/* ── Toasts: what the page says it did, for a moment, in the corner ──────────────────────────
   `note()` is called from everywhere (about 160 places) with the line to show. It keeps no history: a
   toast goes after a few seconds, an error stays until it is closed or ten seconds are up. */
const TOAST_MAX = 3;
const TOAST_MS = 4000;
const TOAST_ERR_MS = 10000;
let toasts = [];                 // newest first: { text, sub, err, at }

/* `line` is what was done; `why` the reason it matters or failed, which then leads and `line` follows in monospace. */
const toastText = (line, isError, why) => ({ text: why || line, sub: why ? line : '', err: !!isError });

/* The list with `item` on top. The same text again replaces the one showing (and restarts its time) instead of stacking, so a
   poll that fails every two seconds shows one toast. */
function toastPush(list, item, now, max = TOAST_MAX) {
  const rest = list.filter(t => !(t.text === item.text && t.sub === item.sub && t.err === item.err));
  return [{ ...item, at: now }, ...rest].slice(0, max);
}

/* A toast is over once its time is up. */
const toastLive = (list, now) => list.filter(t => now - t.at < (t.err ? TOAST_ERR_MS : TOAST_MS));

let toastTimer = null;
function drawToasts() {
  const host = document.getElementById('toasts');
  if (!host) return;
  host.innerHTML = toasts.map((t, i) => `<div class="toast${t.err ? ' err' : ''}" role="${t.err ? 'alert' : 'status'}">` +
    `<div class="toast-body"><div class="toast-text">${esc(t.text)}</div>${t.sub ? `<div class="toast-sub">${esc(t.sub)}</div>` : ''}</div>` +
    `<button type="button" class="tool-btn" data-toast-close="${i}" aria-label="閉じる"><span class="material-symbols-outlined" aria-hidden="true">close</span></button></div>`).join('');
  // A popover is in the top layer: shown again on each note, it sits above a modal dialog that is open (the question dialog calls note()).
  try { if (host.matches(':popover-open')) host.hidePopover(); } catch { /* not supported */ }
  if (!toasts.length) return;
  try { host.showPopover(); } catch { /* not supported: it is fixed in the corner as it is */ }
}

function sweepToasts() {
  clearTimeout(toastTimer);
  toasts = toastLive(toasts, Date.now());
  drawToasts();
  if (!toasts.length) return;
  const next = Math.min(...toasts.map(t => t.at + (t.err ? TOAST_ERR_MS : TOAST_MS)));
  toastTimer = setTimeout(sweepToasts, Math.max(200, next - Date.now()));
}

function note(line, isError, why) {
  toasts = toastPush(toastLive(toasts, Date.now()), toastText(line, isError, why), Date.now());
  sweepToasts();
}

document.addEventListener('click', e => {
  const b = e.target.closest('[data-toast-close]');
  if (!b) return;
  toasts.splice(Number(b.dataset.toastClose), 1);
  sweepToasts();
});
