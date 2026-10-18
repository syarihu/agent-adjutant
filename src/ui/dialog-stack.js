/* ── Dialogs that stack ──────────────────────────────────────────────────────────────────────────────────────────────────────────
   One dialog (#question-dialog), with one layer per level: a question's card, and over it the full diff, the Issue body or a
   report. Only the top layer is shown; the ones below are hidden, not destroyed, so what was typed and where it was scrolled to is
   there when they come back. 「←」 or Escape returns one level, × closes them all. The card is the first user (question-dialog.js);
   the next dialogs push their own layers here. Loaded before question-dialog.js. */

/* The stack itself, with no page in it: the layers in the order they were pushed, the top last. */
function dialogStackCore() {
  const layers = [];
  return {
    layers,
    push(layer) { layers.push(layer); return layers.length; },
    /* The top layer, or null; the stack is empty after the last one. */
    pop() { return layers.pop() || null; },
    /* Every layer, the top first, which is the order they are closed in. */
    closeAll() { return layers.splice(0).reverse(); },
    top: () => layers[layers.length - 1] || null,
    depth: () => layers.length,
    /* A way back is only wanted from the second layer on. */
    backWanted: () => layers.length > 1,
  };
}

/* The head of a layer: a line over the title, then the title. `sub` is markup the caller has escaped. */
const dialogHeadHtml = ({ sub = '', title = '' }) => `${sub ? `<div class="qd-sub">${sub}</div>` : ''}<h2 class="qd-title">${esc(title)}</h2>`;

/* `dialogStack.push({ key, head | { title, sub }, body, foot, gate, opener, onClose })`: `head`, `body` and `foot` are markup the caller
   has escaped (`foot` is optional); `gate` is the key of the gate the layer answers, which its clicks find through `data-qc-gate`;
   `opener` is the element focus returns to when everything closes (the focused one when left out); `onClose(layer)` runs when the layer
   goes, however it goes. */
const dialogStack = (() => {
  const core = dialogStackCore();
  let bound = false;
  let baseOpener = null;
  let seq = 0;
  const dlg = () => document.getElementById('question-dialog');
  const holder = () => dlg().querySelector('.qd-layers');
  const layerEl = key => [...holder().children].find(el => el.dataset.qdKey === key) || null;

  /* What shows the layer on top and says what the dialog is called. */
  function showTop() {
    const top = core.top();
    for (const el of holder().children) el.hidden = el.dataset.qdKey !== top?.key;
    const el = top && layerEl(top.key);
    if (el) {
      dlg().setAttribute('aria-labelledby', el.querySelector('h2')?.id || '');
      el.querySelector('.qd-body')?.focus({ preventScroll: true });
    }
  }

  /* Everything is closed: each layer is told, top first, what it drew goes, and focus goes back to where it came from. */
  function finish(gone) {
    for (const layer of gone) layer.onClose?.(layer);
    holder().replaceChildren();
    const back = baseOpener;
    baseOpener = null;
    if (back?.isConnected) back.focus();
  }

  function closeAll() {
    if (!bound) return;
    const gone = core.closeAll();
    if (dlg().open) dlg().close();
    finish(gone);
  }

  function pop() {
    // From the first layer, a way back is the way out.
    if (core.depth() <= 1) return closeAll();
    const layer = core.pop();
    layerEl(layer.key)?.remove();
    layer.onClose?.(layer);
    showTop();
  }

  /* Bound on first use: the page's element is not there when this script is read. */
  function bind() {
    if (bound) return;
    bound = true;
    const d = dlg();
    d.addEventListener('click', e => {
      if (e.target.closest('[data-qd-back]')) pop();
      else if (e.target.closest('[data-qd-close]')) closeAll();
    });
    d.addEventListener('keydown', e => {
      if (e.key !== 'Escape') return;
      // An IME takes Escape to cancel the conversion (keyCode 229 where compositionend came first): it closes nothing.
      if (e.isComposing || e.keyCode === 229) return e.preventDefault();
      if (core.depth() > 1) { e.preventDefault(); pop(); }
    }, true);
    // Escape in a browser that raises `cancel` without the key first: one level back, and the last level closes with the dialog.
    d.addEventListener('cancel', e => {
      if (core.depth() > 1) { e.preventDefault(); pop(); }
    });
    // Closed by the browser rather than by `closeAll`: the same bookkeeping.
    // Not when the dialog was opened again since: a `close` event comes after the call that closed it.
    d.addEventListener('close', () => { if (core.depth() && !d.open) finish(core.closeAll()); });
  }

  function push(layer) {
    bind();
    if (layerEl(layer.key)) return false;
    const n = ++seq;
    if (!core.depth()) baseOpener = layer.opener || document.activeElement;
    const back = core.depth() > 0
      ? '<button type="button" class="m3-icon-button qd-back" data-qd-back aria-label="戻る" title="戻る"><span class="material-symbols-outlined" aria-hidden="true">arrow_back</span></button>' : '';
    const head = (layer.head ?? dialogHeadHtml(layer)).replace('<h2 class="qd-title">', `<h2 class="qd-title" id="qd-title-${n}">`);
    holder().insertAdjacentHTML('beforeend', `<section class="qd-layer" data-qd-key="${esc(layer.key)}"${layer.gate ? ` data-qc-gate="${esc(layer.gate)}"` : ''}>`
      + `<header class="qd-head">${back}<div class="qd-head-main">${head}</div>`
      + '<button type="button" class="m3-icon-button qd-x" data-qd-close aria-label="閉じる" title="閉じる"><span class="material-symbols-outlined" aria-hidden="true">close</span></button></header>'
      + `<div class="qd-body" tabindex="0" role="region" aria-label="内容">${layer.body}</div>`
      + `${layer.foot ? `<footer class="qd-foot">${layer.foot}</footer>` : ''}</section>`);
    core.push(layer);
    if (!dlg().open) dlg().showModal();
    showTop();
    return true;
  }

  return {
    push, pop, closeAll, layerEl,
    isOpen: () => bound && dlg().open && core.depth() > 0,
    depth: () => core.depth(),
    has: key => core.layers.some(l => l.key === key),
    /* The layers, bottom first. */
    layers: () => core.layers,
  };
})();
