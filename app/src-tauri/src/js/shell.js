// Runs at the start of every page the window loads (and again from the app when the appearance
// changes). The strip under the title bar is on every page of the window; the rest only does anything
// on the board's own origin: the loopback http page the server serves.
(function () {
  // The window's title bar is an overlay: the page runs up to the top edge, under the traffic
  // lights, and the system no longer moves the window from there. This strip, as tall as the title
  // bar, does. `--app-titlebar` is its height in the page's pixels (the page zoom scales those, not
  // the title bar), and 0 in full screen, where the system shows its title bar only on hover; the
  // board's app-only styles (src/ui/app-shell.css) lay the page out below it.
  const TITLE_BAR_HEIGHT = 28;
  const invoke = (cmd, args) => window.__TAURI_INTERNALS__
    ? window.__TAURI_INTERNALS__.invoke(cmd, args).catch(() => {})
    : undefined;
  let frame = __FRAME__;
  let strip = null;
  const layout = () => {
    const root = document.documentElement;
    if (!root) return;
    const height = frame.fullscreen ? 0 : TITLE_BAR_HEIGHT / (frame.zoom || 1);
    root.style.setProperty('--app-titlebar', `${height}px`);
    if (strip) strip.hidden = !!frame.fullscreen;
  };
  window.__adjFrame = next => {
    frame = next;
    layout();
  };
  const addStrip = () => {
    if (strip || !document.body) return;
    strip = document.createElement('div');
    strip.id = 'app-titlebar';
    strip.setAttribute('aria-hidden', 'true');
    strip.style.cssText = 'position:fixed;top:0;left:0;right:0;height:var(--app-titlebar,0px);z-index:35;';
    // A press drags the window. A double-click does what System Settings says (zoom, minimize or
    // nothing): on its release and only if the pointer stayed put, as on a real title bar.
    let downX = null;
    let downY = null;
    strip.addEventListener('mousedown', event => {
      if (event.button !== 0) return;
      event.preventDefault();
      if (event.detail === 2) {
        downX = event.clientX;
        downY = event.clientY;
      } else if (event.detail === 1) {
        invoke('plugin:window|start_dragging');
      }
    });
    strip.addEventListener('mouseup', event => {
      if (event.button !== 0 || event.detail !== 2) return;
      if (event.clientX === downX && event.clientY === downY) invoke('title_bar_double_click');
      downX = null;
      downY = null;
    });
    document.body.appendChild(strip);
    layout();
  };
  layout();
  if (document.body) addStrip();
  else document.addEventListener('DOMContentLoaded', addStrip);

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
      invoke('zoom', { direction: 'in' });
    }, true);
  }
})();
