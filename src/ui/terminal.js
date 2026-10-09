/* A tmux session's terminal, inside any container. The component only assumes the container
   has a size of its own: it fills it and follows it. The task panel and the work view's middle
   mount it; a page of its own for a session can mount the same. xterm.js is served by the
   resident server and loaded on first use. */
let xtermLoading = null;
function loadXterm(base = BASE) {
  if (window.Terminal && window.FitAddon && window.Unicode11Addon) return Promise.resolve();
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

const TERMINAL_ENDS = {
  1000: '切り離しました',
  4404: 'セッションが見つからないか、tmux 上にありません',
  1006: '接続できませんでした',
};
function terminalEndText(code, reason) {
  if (code === 4500) return `tmux の操作に失敗しました: ${reason || '理由は不明です'}`;
  return TERMINAL_ENDS[code] || `接続が閉じました（${code}）`;
}

/* `base` is the board's path; a view across boards, where `BASE` is empty, names the board of
   the session. */
function mountSessionTerminal(container, { sessionId, onEnd, base = BASE, focus = true } = {}) {
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
      if (!disposed) { container.textContent = e.message; onEnd?.(1006); }
      return;
    }
    if (disposed) return;
    term = new Terminal({
      allowProposedApi: true,
      scrollback: 1000,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 13,
    });
    fit = new FitAddon.FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon.Unicode11Addon());
    term.unicode.activeVersion = '11';
    term.open(container);
    fit.fit();

    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const query = `token=${encodeURIComponent(TOKEN)}&cols=${term.cols}&rows=${term.rows}`;
    ws = new WebSocket(`${scheme}//${location.host}${base}/api/sessions/${encodeURIComponent(sessionId)}/terminal?${query}`);
    ws.binaryType = 'arraybuffer';
    const open = () => ws && ws.readyState === WebSocket.OPEN;
    // A resize while connecting was not sent; the size now is what the PTY should have.
    ws.onopen = () => ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    ws.onmessage = e => { if (e.data instanceof ArrayBuffer) term.write(new Uint8Array(e.data)); };
    ws.onclose = e => {
      if (disposed) return;
      line(terminalEndText(e.code, e.reason));
      onEnd?.(e.code);
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
const sessionOfTask = task => task && !scopeAll() && (state.sessions || []).find(s =>
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
