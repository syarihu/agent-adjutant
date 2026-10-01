/* A tmux session's terminal, inside any container. The component only assumes the container
   has a size of its own: it fills it and follows it. The board's セッション view mounts it; a
   page of its own for a session can mount the same. xterm.js is served by the resident server
   and loaded on first use. */
let xtermLoading = null;
function loadXterm() {
  if (window.Terminal && window.FitAddon && window.Unicode11Addon) return Promise.resolve();
  if (xtermLoading) return xtermLoading;
  const token = encodeURIComponent(TOKEN);
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = `${BASE}/vendor/xterm.css?token=${token}`;
  document.head.appendChild(link);
  xtermLoading = new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = `${BASE}/vendor/xterm.js?token=${token}`;
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

/* `onReady` is called once, when the first output has arrived: the point from which the
   terminal is on screen and whatever else the page reads can go ahead. */
function mountSessionTerminal(container, { sessionId, onEnd, onReady } = {}) {
  container.classList.add('adj-terminal');
  let term = null;
  let fit = null;
  let ws = null;
  let observer = null;
  let frame = 0;
  let disposed = false;
  let ready = false;
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
      await loadXterm();
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
    ws = new WebSocket(`${scheme}//${location.host}${BASE}/api/sessions/${encodeURIComponent(sessionId)}/terminal?${query}`);
    ws.binaryType = 'arraybuffer';
    const open = () => ws && ws.readyState === WebSocket.OPEN;
    // A resize while connecting was not sent; the size now is what the PTY should have.
    ws.onopen = () => ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    ws.onmessage = e => {
      const first = !ready;
      ready = true;
      // Once xterm has drawn the first frame: `write` calls back when the data is parsed and
      // `onRender` fires inside xterm's animation frame, before the browser paints, so the
      // page waits one more turn. Kept from the socket: a page that fails to draw its sidebar
      // must not lose the output or end the handler. A frame that draws nothing leaves the
      // page to its own fallback.
      const done = () => {
        if (!first || disposed) return;
        const drawn = term.onRender(() => {
          drawn.dispose();
          setTimeout(() => {
            if (disposed) return;
            try { onReady?.(); } catch (err) { console.error(err); }
          }, 0);
        });
      };
      if (e.data instanceof ArrayBuffer) term.write(new Uint8Array(e.data), done);
      else done();
    };
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
    term.focus();
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
const sessionOfTask = task => task && (state.sessions || []).find(s =>
  // The worktree stands in only for a session with no task of its own: one that belongs to
  // another task is not this card's, even where a worktree was reused.
  s.kind === 'worker' && (s.task ? s.task === task.id : !!task.worktree && s.worktree === task.worktree));
const readySessionOfTask = task => {
  const s = sessionOfTask(task);
  return boardTerminalReady(s) ? s : null;
};
function openTaskTerminal(taskId) {
  const task = (state.tasks || []).find(t => t.id === taskId);
  const s = readySessionOfTask(task);
  if (s) openSessionsView(s.id, { from: { view, taskId } });
}
