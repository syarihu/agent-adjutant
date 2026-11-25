/* ── Motion: short, playful, never in the way ────────────────────────────────────────────────────────────────────────────────────
   What the page animates (a view rising, a card arriving, a card changing stage, a badge counting up, a toast, a dialog) shares this
   file's parts. The rules every motion keeps: nothing delays a click or hides content longer than the motion lasts, and with
   `prefers-reduced-motion: reduce` (or no Element.animate) nothing is added at all: no class, no clone, no animate() call
   (`motionCalm`). The keyframes that are classes live in motion.css, inside a no-preference query.
   Nothing here touches the DOM when the script is read, so the tests can load it. */

/* Whether the page must stay still: reduced motion is asked for, or the engine cannot animate. */
function motionCalm() {
  if (typeof Element === 'undefined' || typeof Element.prototype.animate !== 'function') return true;
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/* The easing of a motion token. An engine that cannot parse the spring's linear() would make animate() throw: it gets a plain ease. */
function motionEasing(kind = 'spatial') {
  const css = getComputedStyle(document.documentElement);
  const token = css.getPropertyValue(`--md-sys-motion-spring-fast-${kind}`).trim();
  return token && CSS.supports('transition-timing-function', token) ? token : 'ease';
}
const springEasing = () => motionEasing('spatial');

/* `el.animate(frames, options)` unless the page must stay still (then null); a failure of the engine is not the page's failure. */
function motionAnimate(el, frames, options) {
  if (!el || motionCalm()) return null;
  try { return el.animate(frames, options); } catch { return null; }
}

/* Plays the keyframes of class `cls` on `el` once, again if it is already running: the class is taken off, the page is made to
   notice, and it is put back. It is taken off at animationend, or after `ms` plus a little when that event never comes (the element
   was hidden, the animation cancelled), so a stale class cannot replay later. */
function replayClass(el, cls, ms = 400) {
  if (!el || motionCalm()) return;
  el.classList.remove(cls);
  void el.offsetWidth;
  el.classList.add(cls);
  let timer = null;
  const end = () => {
    clearTimeout(timer);
    el.removeEventListener('animationend', onEnd);
    el.classList.remove(cls);
  };
  const onEnd = e => { if (e.target === el) end(); };
  el.addEventListener('animationend', onEnd);
  timer = setTimeout(end, ms + 100);
}

/* The keys of `nextKeys` that were not in `prev` (a Set), for what has just arrived. Empty when there is no earlier set to compare
   with (the first drawing, or one that was reset): everything on the page would be "new". */
function diffKeys(prev, nextKeys) {
  // An empty earlier set is a first drawing too: a board switch draws an empty state before its data arrives, and that must not pop everything in.
  if (!prev || !prev.size) return [];
  return [...nextKeys].filter(k => !prev.has(k));
}

/* What changed between two maps of key → stage: `appeared` (keys not there before), `moved` ({ key, from, to } of those whose stage
   differs) and `finished` (the keys that moved into 'done'). Keys that went are ignored. Empty when there is no earlier map. */
function stageDiff(prev, next) {
  // Empty earlier map: see diffKeys.
  const out = { appeared: [], moved: [], finished: [] };
  if (!prev || !prev.size || !next) return out;
  for (const [key, to] of next) {
    if (!prev.has(key)) { out.appeared.push(key); continue; }
    const from = prev.get(key);
    if (from === to) continue;
    out.moved.push({ key, from, to });
    if (to === 'done') out.finished.push(key);
  }
  return out;
}

/* What a drawing remembers between two of its drawings: `motionTrack(name, sig, next)` stores `next` (a Map) and gives back the
   previous one, or null when `sig` (what the drawing is of: the organisation, the filter, the view) is not the one it was stored
   with. A change of `sig` is a reset, so switching what is shown never plays as if things had arrived. `motionForget` is a reset. */
const motionMemo = new Map();
function motionTrack(name, sig, next) {
  const had = motionMemo.get(name);
  motionMemo.set(name, { sig, value: next });
  return had && had.sig === sig ? had.value : null;
}
function motionForget(name) { motionMemo.delete(name); }

/* A count that has just gone up: true when `n` is more than the last one stored under `name` for the same `sig`. The first value
   after a reset is never one. */
function motionCountedUp(name, sig, n) {
  const had = motionMemo.get(name);
  motionMemo.set(name, { sig, value: n });
  return !!had && had.sig === sig && n > had.value;
}

/* The fixed layer that clones and bursts fly in: over the page, ignoring the pointer, and not read out. */
function motionLayer() {
  let el = document.getElementById('motion-layer');
  if (!el) {
    el = document.createElement('div');
    el.id = 'motion-layer';
    el.setAttribute('aria-hidden', 'true');
    document.body.appendChild(el);
  }
  return el;
}

/* Whether a rect is on screen at all (an element in a column scrolled out of sight is not worth flying to). */
const motionOnScreen = r => !!r && r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;

/* A screen comes into view: it rises a little while it fades in. `els` are the containers shown (hidden ones are skipped). `fade`
   makes it opacity only, for what hosts the terminal: a transform there would show in xterm's fit. */
function motionEnter(els, { fade = false } = {}) {
  const easing = motionEasing('effects');
  for (const el of els) {
    // getClientRects, not offsetParent: a shown panel that is position: fixed (the narrow layout) has no offsetParent either.
    if (!el || el.hidden || !el.getClientRects().length) continue;
    const from = fade ? { opacity: 0 } : { opacity: 0, transform: 'translateY(12px)' };
    const to = fade ? { opacity: 1 } : { opacity: 1, transform: 'none' };
    motionAnimate(el, [from, to], { duration: 500, easing });
  }
}

/* The cards or rows that came to the board pop in. */
function motionPopIn(els) {
  const easing = motionEasing('effects');
  for (const el of els) motionAnimate(el, [{ opacity: 0, transform: 'scale(.94)' }, { opacity: 1, transform: 'none' }], { duration: 220, easing });
}

/* A card that changed stage flies from where it was to where it is now (`from`, `to`: rects). It flies as a clone in the fixed layer,
   since a column scrolls and would cut it off; the card itself is already in its place, shown again as the clone lands. `ghost` is
   what stands for a row, whose grid cannot be cloned out of its list. Returns how long the flight takes (0: none). */
function motionFly(el, from, to, ghost = false) {
  const dx = from.left - to.left, dy = from.top - to.top;
  if (!dx && !dy) return 0;
  const duration = 350;
  const clone = ghost ? document.createElement('div') : el.cloneNode(true);
  if (ghost) {
    clone.className = 'mo-ghost-row';
    clone.textContent = el.querySelector('.title')?.textContent || '';
  } else {
    clone.removeAttribute('id');
    for (const x of clone.querySelectorAll('[id]')) x.removeAttribute('id');
    clone.classList.remove('selected', 'flash');
  }
  clone.setAttribute('aria-hidden', 'true');
  clone.setAttribute('inert', '');
  clone.removeAttribute('tabindex');
  for (const x of clone.querySelectorAll('[tabindex]')) x.removeAttribute('tabindex');
  Object.assign(clone.style, { position: 'fixed', left: `${to.left}px`, top: `${to.top}px`, width: `${to.width}px`, height: `${to.height}px`, margin: '0', pointerEvents: 'none', boxSizing: 'border-box' });
  motionLayer().appendChild(clone);
  const easing = motionEasing('spatial');
  const flight = motionAnimate(clone, [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration, easing });
  if (!flight) { clone.remove(); return 0; }
  // The real card shows as the clone lands, and is never hidden past the flight.
  motionAnimate(el, [{ opacity: 0 }, { opacity: 0, offset: .7 }, { opacity: 1 }], { duration });
  const end = () => clone.remove();
  flight.onfinish = end;
  flight.oncancel = end;
  setTimeout(end, duration + 300);
  return duration;
}

/* A card that was finished: a check that bounces and a few bits of paper, in a container over the card that is gone after. `delay`
   waits for the card's flight to land. */
function motionCelebrate(rect, delay = 0) {
  if (motionCalm()) return;
  const box = document.createElement('div');
  box.className = 'mo-burst';
  Object.assign(box.style, { left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px`, height: `${rect.height}px` });
  const check = document.createElement('span');
  check.className = 'material-symbols-outlined mo-burst-check';
  check.textContent = 'check_circle';
  box.appendChild(check);
  const colors = ['primary', 'success', 'warning', 'error'];
  const confetti = [];
  for (let i = 0; i < 8; i++) {
    const bit = document.createElement('span');
    bit.className = 'mo-confetti';
    bit.style.background = `var(--md-sys-color-${colors[i % colors.length]})`;
    box.appendChild(bit);
    confetti.push(bit);
  }
  motionLayer().appendChild(box);
  const easing = motionEasing('spatial');
  motionAnimate(check, [
    { transform: 'scale(.4) rotate(-20deg)', opacity: 0 },
    { transform: 'scale(1.2) rotate(4deg)', opacity: 1, offset: .6 },
    { transform: 'none', opacity: 1 },
  ], { duration: 420, easing, delay, fill: 'backwards' });
  confetti.forEach((bit, i) => {
    const a = Math.PI * 2 * i / 8 + Math.random() * .4;
    const r = 30 + Math.random() * 24;
    motionAnimate(bit, [
      { transform: 'translate(0, 0) rotate(0deg)', opacity: 1 },
      { transform: `translate(${Math.cos(a) * r}px, ${Math.sin(a) * r - 10}px) rotate(${Math.random() * 360}deg)`, opacity: 0 },
    ], { duration: 700, easing: 'cubic-bezier(.2, .7, .3, 1)', delay, fill: 'both' });
  });
  setTimeout(() => box.remove(), delay + 900);
}
