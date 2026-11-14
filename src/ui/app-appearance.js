/* The desktop app's window, in the display settings: what is behind the board (the macOS material, which blurs it,
   or nothing) and how opaque the board's surfaces stay over it. Only the app has these: its init script
   (app/src-tauri/src/js/shell.js) stamps `data-shell="app"` on <html> and puts the current values in
   `window.__adjAppearance`. A browser has neither, so the section stays hidden and nothing here runs; so does an
   older app that does not know the `set_appearance` command yet. */
function appAppearanceSupported(root, win) {
  return root?.getAttribute('data-shell') === 'app'
    && !!win.__adjAppearance && typeof win.__adjAppearance === 'object'
    && typeof win.__TAURI_INTERNALS__?.invoke === 'function';
}

const appAppearancePercent = alpha => Math.round(Number(alpha) * 100);

function setupAppAppearance(doc, win) {
  const section = doc.getElementById('display-app');
  const root = doc.documentElement;
  if (!section || !appAppearanceSupported(root, win)) return false;
  const slider = doc.getElementById('display-app-opacity');
  const output = doc.getElementById('display-app-opacity-value');
  const radios = [...section.querySelectorAll('input[name="display-app-background"]')];
  const error = doc.getElementById('display-app-error');
  const showError = message => {
    if (!error) return;
    error.textContent = message;
    error.hidden = !message;
  };

  // Changes go to the app one at a time, and steps of a drag that pile up while one is on its way are folded into the
  // latest: the app answers each by putting the result back on the page (`adj-appearance`).
  let next = null;
  let sending = false;
  const pump = async () => {
    sending = true;
    while (next) {
      const args = next;
      next = null;
      try {
        await win.__TAURI_INTERNALS__.invoke('set_appearance', args);
        // A step that is not saved says nothing about whether the last save went through.
        if (args.persist || args.background) showError('');
      } catch (e) {
        // The window already shows the change; it is only not kept for the next start.
        showError(`設定を保存できませんでした（${e}）。次にアプリを起動したときには戻ります`);
      }
    }
    sending = false;
  };
  const send = args => {
    // The latest step decides whether to save: the slider's last one is sent with `persist`, and the app always saves a
    // change of background whatever it says.
    next = { ...next, ...args, persist: !!args.persist };
    if (!sending) pump();
  };

  // While the person drags (or steps it with the keys and changes are still on their way), the slider is theirs: a value
  // the app echoes back from an earlier step must not move it.
  let dragging = false;
  const draw = appearance => {
    for (const radio of radios) radio.checked = radio.value === appearance.background;
    slider.min = String(appAppearancePercent(appearance.minAlpha ?? 0.3));
    slider.max = String(appAppearancePercent(appearance.maxAlpha ?? 1));
    if (!dragging && !sending && !next) slider.value = String(appAppearancePercent(appearance.alpha));
    output.textContent = `${slider.value}%`;
  };

  for (const radio of radios) {
    radio.addEventListener('change', () => {
      if (radio.checked) send({ background: radio.value, persist: true });
    });
  }
  slider.addEventListener('pointerdown', () => { dragging = true; });
  for (const end of ['pointerup', 'pointercancel']) slider.addEventListener(end, () => { dragging = false; });
  slider.addEventListener('input', () => {
    const alpha = Number(slider.value) / 100;
    output.textContent = `${slider.value}%`;
    // Shown at once; the app saves it when the slider is let go.
    root.style.setProperty('--app-alpha', String(alpha));
    send({ opacity: alpha, persist: false });
  });
  slider.addEventListener('change', () => {
    dragging = false;
    send({ opacity: Number(slider.value) / 100, persist: true });
  });
  win.addEventListener('adj-appearance', e => draw(e.detail || win.__adjAppearance));

  draw(win.__adjAppearance);
  section.hidden = false;
  return true;
}

setupAppAppearance(document, window);
