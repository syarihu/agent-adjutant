// Runs at the start of every page the window loads (and again from the app when the appearance
// changes). It only does anything on the board's own origin: the loopback http page the server serves.
(function () {
  const onBoard = location.protocol === 'http:'
    && ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
  if (!onBoard) return;

  // The attribute is what src/ui/app-shell.css is scoped to; the browser never has it.
  const apply = appearance => {
    const root = document.documentElement;
    if (!root) return false;
    root.setAttribute('data-shell', 'app');
    root.style.setProperty('--app-alpha', String(appearance.alpha));
    return true;
  };

  // The app calls this when a page loads and whenever the appearance changes (its menu, or the
  // board's display settings). `window.__adjAppearance` is what the board's display settings read
  // (src/ui/app-appearance.js); they hear a change through the `adj-appearance` event.
  window.__adjShell = appearance => {
    window.__adjAppearance = appearance;
    if (apply(appearance)) {
      window.dispatchEvent(new CustomEvent('adj-appearance', { detail: appearance }));
    }
  };
  window.__adjAppearance = __APPEARANCE__;

  // An initialization script can run before <html> exists: wait for it, so the first paint is
  // already translucent, and settle once more when the document is done.
  if (!apply(window.__adjAppearance)) {
    const wait = new MutationObserver(() => {
      if (apply(window.__adjAppearance)) wait.disconnect();
    });
    wait.observe(document, { childList: true });
  }
  document.addEventListener('DOMContentLoaded', () => apply(window.__adjAppearance));

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
