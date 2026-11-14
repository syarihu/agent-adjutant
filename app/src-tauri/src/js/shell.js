// Runs at the start of every page the window loads (and again from the app when the translucency
// changes). It only does anything on the board's own origin: the loopback http page the server serves.
(function () {
  const onBoard = location.protocol === 'http:'
    && ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
  if (!onBoard) return;

  // The attribute is what src/ui/app-shell.css is scoped to; the browser never has it.
  const apply = alpha => {
    const root = document.documentElement;
    if (!root) return false;
    root.setAttribute('data-shell', 'app');
    root.style.setProperty('--app-alpha', String(alpha));
    return true;
  };

  window.__adjShell = alpha => {
    window.__adjAlpha = alpha;
    apply(alpha);
  };
  window.__adjAlpha = __ALPHA__;

  // An initialization script can run before <html> exists: wait for it, so the first paint is
  // already translucent, and settle once more when the document is done.
  if (!apply(window.__adjAlpha)) {
    const wait = new MutationObserver(() => {
      if (apply(window.__adjAlpha)) wait.disconnect();
    });
    wait.observe(document, { childList: true });
  }
  document.addEventListener('DOMContentLoaded', () => apply(window.__adjAlpha));

  // Cmd++ zooms in, as Cmd+= does. The menu's shortcut names a physical key (=), so the key that
  // types + (Shift+= on a US keyboard, Shift+; on a Japanese one) is caught here, before the page.
  if (window.__TAURI_INTERNALS__) {
    window.addEventListener('keydown', event => {
      if (event.key !== '+' || !event.metaKey || event.ctrlKey || event.altKey) return;
      event.preventDefault();
      event.stopPropagation();
      window.__TAURI_INTERNALS__.invoke('zoom', { direction: 'in' }).catch(() => {});
    }, true);
  }
})();
