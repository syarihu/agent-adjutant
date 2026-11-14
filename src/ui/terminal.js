/* A tmux session's terminal, inside any container. The component only assumes the container
   has a size of its own: it fills it and follows it. The task panel and the work view's middle
   mount it; a page of its own for a session can mount the same. xterm.js is served by the
   resident server and loaded on first use. */
let xtermLoading = null;
function loadXterm(base = BASE) {
  if (window.Terminal && window.FitAddon && window.Unicode11Addon && window.WebLinksAddon) return Promise.resolve();
  if (xtermLoading) return xtermLoading;
  const token = encodeURIComponent(TOKEN);
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = `${base}/vendor/xterm.css?token=${token}`;
  document.head.appendChild(link);
  xtermLoading = new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = `${base}/vendor/xterm.js?token=${token}`;
    script.onload = resolve;
    script.onerror = () => reject(new Error('xterm.js を読み込めませんでした'));
    document.head.appendChild(script);
  });
  // A failed load is tried again on the next open.
  xtermLoading.catch(() => { xtermLoading = null; });
  return xtermLoading;
}

/* In the desktop app (`data-shell="app"`, see app-shell.css) the terminal paints no background of
   its own, so the window's translucency shows through it; its dark backdrop is the container's,
   mixed down to `--app-alpha` there. The cursor's accent defaults to the background, which would
   leave the character under a block cursor invisible, so it is given xterm's default black. A
   browser gets no options from here and renders as it always did. */
function appShellTerminalOptions(root = document.documentElement) {
  if (root?.getAttribute('data-shell') !== 'app') return {};
  return { allowTransparency: true, theme: { background: 'rgba(0, 0, 0, 0)', cursorAccent: '#000000' } };
}

/* A browser extension that runs before the page (Vimium's insert mode) can take Escape from a
   focused terminal: it blurs the text area and the key never reaches xterm, nor the page. All
   the page sees is the text area losing the focus to nothing, with no press elsewhere and the
   window still focused. Said once, so the person knows where the key went. Neither the focus
   nor the key is put back: a guessed ESC would interrupt the agent. */
let termPointerAt = 0;
document.addEventListener('pointerdown', e => {
  if (!e.target?.closest?.('.adj-terminal')) termPointerAt = Date.now();
}, true);
const terminalLostToNothing = (e, { now = Date.now(), pointerAt = termPointerAt, hasFocus = document.hasFocus() } = {}) =>
  !!e.target?.classList?.contains('xterm-helper-textarea') && !!e.target.closest?.('.adj-terminal')
  && e.relatedTarget == null && hasFocus && now - pointerAt > 500;
// Settled a task later: the page's own disposal (another row, another view, the panel closed) and
// hiding (another tab of the panel) blur the text area too, and leave it gone or hidden by then. A
// window that lost the focus meanwhile does not spend the one note either.
const terminalStillThere = (ta, active = document.activeElement) =>
  document.hasFocus() && ta.isConnected && !!ta.closest('.adj-terminal')
  && (ta.closest('.adj-terminal').checkVisibility?.() ?? true) && !active?.closest?.('.adj-terminal');
let terminalTakenNoted = false;
document.addEventListener('focusout', e => {
  if (terminalTakenNoted || !terminalLostToNothing(e)) return;
  const ta = e.target;
  setTimeout(() => {
    if (terminalTakenNoted || !terminalStillThere(ta)) return;
    terminalTakenNoted = true;
    note('ターミナルのフォーカスが外れました', true,
      'ブラウザの拡張機能（Vimium など）が Escape を先に取ると、エージェントには届きません。拡張機能の設定でこのボードを除外してください');
  });
}, true);

const TERMINAL_ENDS = {
  1000: '切り離しました',
  4410: 'ウィンドウは終わっています',
  4404: 'セッションが見つからないか、tmux 上にありません',
  1006: '接続できませんでした',
};
function terminalEndText(code, reason) {
  // Not a close code: the page could not start the terminal at all, and says why.
  if (code === 0) return reason || '接続できませんでした';
  if (code === 4500) return `tmux の操作に失敗しました: ${reason || '理由は不明です'}`;
  return TERMINAL_ENDS[code] || `接続が閉じました（${code}）`;
}

/* Links inside the terminal are opened only with the platform modifier (Cmd on macOS, Ctrl
   elsewhere) so that plain clicks and text selection are left alone. tmux forwards OSC 8
   hyperlinks only when the client has the feature, while plain text URLs are found by the
   addon. A URL that tmux broke across two screen lines is not joined (xterm only joins lines it
   wrapped itself), and when tmux has the mouse on, the click also reaches tmux, where it only
   selects the pane. */
// The addon's own expression with one balanced pair of parentheses allowed inside, so a URL such
// as a Wikipedia title (`/wiki/Rust_(programming_language)`) is not cut at its `(`. A `)` with no
// `(` before it in the URL, as in `(see https://…)`, still ends it.
const TERMINAL_URL = /(https?|HTTPS?):\/\/(?:[^\s"'!*(){}|\\^<>`]|\([^\s"'!*(){}|\\^<>`]*\))*(?:[^\s"':,.!?{}|\\^~\[\]`()<>]|\([^\s"'!*(){}|\\^<>`]*\))/;
function terminalLinkOpens(event, uri, mac = IS_MAC) {
  const modifier = mac ? !!event?.metaKey : !!event?.ctrlKey;
  if (!modifier || !uri) return false;
  try {
    const u = new URL(uri);
    return u.protocol === 'http:' || u.protocol === 'https:';
  } catch {
    return false;
  }
}
// Takes the event and the URI alone: whatever else a caller passes (xterm's range) is not a platform.
function openTerminalLink(event, uri) {
  if (!terminalLinkOpens(event, uri)) return;
  window.open(uri, '_blank', 'noopener,noreferrer');
}

/* `base` is the board's path; a view across boards, where `BASE` is empty, names the board of
   the session. `path` is the socket's path when it is not a board session's (a PR's reader: it may carry a query). */
function mountSessionTerminal(container, { sessionId, path = null, onEnd, base = BASE, focus = true } = {}) {
  container.classList.add('adj-terminal');
  let term = null;
  let fit = null;
  let ws = null;
  let observer = null;
  let frame = 0;
  let disposed = false;
  const encoder = new TextEncoder();

  const scheduleFit = () => {
    if (frame || disposed) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      // A hidden container has no size to fit to.
      if (!disposed && fit && container.clientWidth > 0 && container.clientHeight > 0) fit.fit();
    });
  };
  const line = text => term && term.write(`\r\n\x1b[2m[${text}]\x1b[0m\r\n`);

  (async () => {
    try {
      await loadXterm(base);
    } catch (e) {
      if (!disposed) { container.textContent = e.message; onEnd?.(0, e.message); }
      return;
    }
    if (disposed) return;
    term = new Terminal({
      allowProposedApi: true,
      scrollback: 1000,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 13,
      linkHandler: {
        activate: openTerminalLink,
        allowNonHttpProtocols: false,
      },
      ...appShellTerminalOptions(),
    });
    fit = new FitAddon.FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon.Unicode11Addon());
    term.loadAddon(new WebLinksAddon.WebLinksAddon(openTerminalLink, { urlRegex: TERMINAL_URL }));
    term.unicode.activeVersion = '11';
    term.open(container);
    // On macOS Cmd+K opens the session search even here; every other key, Ctrl+K (readline's kill-line) included, is the agent's.
    term.attachCustomKeyEventHandler(e => !(IS_MAC && isPaletteShortcut(e, true) && openPaletteFromKey(e)));
    fit.fit();

    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const query = `token=${encodeURIComponent(TOKEN)}&cols=${term.cols}&rows=${term.rows}`;
    const socket = path || `${base}/api/sessions/${encodeURIComponent(sessionId)}/terminal`;
    ws = new WebSocket(`${scheme}//${location.host}${socket}${socket.includes('?') ? '&' : '?'}${query}`);
    ws.binaryType = 'arraybuffer';
    const open = () => ws && ws.readyState === WebSocket.OPEN;
    // A resize while connecting was not sent; the size now is what the PTY should have.
    ws.onopen = () => ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    ws.onmessage = e => { if (e.data instanceof ArrayBuffer) term.write(new Uint8Array(e.data)); };
    ws.onclose = e => {
      if (disposed) return;
      line(terminalEndText(e.code, e.reason));
      onEnd?.(e.code, e.reason);
    };
    term.onData(data => { if (open()) ws.send(encoder.encode(data)); });
    // Mouse reports and the like arrive as a string of bytes, not of characters.
    term.onBinary(data => {
      if (open()) ws.send(Uint8Array.from(data, c => c.charCodeAt(0) & 0xff));
    });
    term.onResize(({ cols, rows }) => {
      if (open()) ws.send(JSON.stringify({ type: 'resize', cols, rows }));
    });
    observer = new ResizeObserver(scheduleFit);
    observer.observe(container);
    if (focus) term.focus();
  })();

  return {
    focus() { term?.focus(); },
    // Keys typed for the person, with no Enter: they finish the sentence. False when the socket is not open.
    type(text) {
      if (!ws || ws.readyState !== WebSocket.OPEN) return false;
      ws.send(encoder.encode(text));
      return true;
    },
    // Text pasted the way a person's paste is: xterm turns line breaks into CR and wraps the text in
    // the bracketed-paste markers when the terminal asked for them (under tmux it always does, and tmux
    // passes them on only to a program that asked). False when the socket is not open.
    paste(text) {
      if (!term || !ws || ws.readyState !== WebSocket.OPEN) return false;
      term.paste(text);
      return true;
    },
    // The Enter key alone.
    enter() { return this.type('\r'); },
    fit: scheduleFit,
    // The last lines on screen, for a panel to show once the pane behind them is gone. Read
    // before `dispose`, which takes the buffer with it.
    snapshot() {
      if (!term) return [];
      const buffer = term.buffer.active;
      const lines = [];
      for (let i = buffer.length - 1; i >= 0 && lines.length < 40; i--) {
        const text = buffer.getLine(i)?.translateToString(true) ?? '';
        if (text.trim()) lines.unshift(text);
      }
      return lines;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      observer?.disconnect();
      // Closing the socket is the detach: the session itself keeps running.
      try { ws?.close(); } catch {}
      term?.dispose();
      container.classList.remove('adj-terminal');
      container.textContent = '';
    },
  };
}

/* A terminal is offered only where it can open: the resident server has tmux, and the session
   is alive in a tmux window. Where it cannot, there is no button at all. */
const boardTerminalReady = s =>
  !!(state.boardTerminal?.available && s?.present && s.terminal?.backend === 'tmux' && s.terminal.window);
const sessionOfTask = task => task && !scopeAll() && (stateNow().sessions || []).find(s =>
  // The worktree stands in only for a session with no task of its own: one that belongs to
  // another task is not this card's, even where a worktree was reused.
  s.kind === 'worker' && (s.task ? s.task === task.id : !!task.worktree && s.worktree === task.worktree));
/* The task a worker belongs to on this board, by the same rule: its own `task`, else the worktree
   a task names. */
const taskOfSession = s => !!s && s.kind === 'worker' && (state.tasks || []).find(t =>
  (!scopeAll() || t._slug === s._slug) && (s.task ? t.id === s.task : !!t.worktree && t.worktree === s.worktree)) || null;
const readySessionOfTask = task => {
  const s = sessionOfTask(task);
  return boardTerminalReady(s) ? s : null;
};
