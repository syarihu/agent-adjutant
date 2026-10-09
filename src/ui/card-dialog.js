/* 「拡大」: one card of the task panel, alone, in a large dialog. The dialog copies the panel's own DOM
   (it does not draw the card again from data), so it reads exactly as the panel does, and it follows
   the panel's redraws while it is open. A card that belongs to a gate brings that gate's answer
   controls along; answering closes the dialog. */

const cardDialog = {
  subject: null, pane: null, section: null, gate: null, opener: null,
  sig: '', held: false, quiet: false, gone: false, pending: false, observer: null,
  expectControls: false, token: 0, pressing: false, answering: false, gateGone: false,
};
const CARD_DIALOG_GONE = 'パネルからこのカードがなくなりました（最後に表示した内容です）';
const CARD_DIALOG_GATE_GONE = 'この gate への答えはパネルから消えました（最後に表示した内容です）';
const cardDialogEl = () => document.getElementById('card-dialog');

/* What a card says about itself, drawn into the panel's markup by task-view.js and decide.js. */
const expandAttrs = (section, g = null) =>
  ` data-expand="${section}"` + (g ? ` data-expand-gate="${esc(gateRef(g))}"` : '');
const expandBtnHtml = label =>
  `<button type="button" class="tool-btn expand-btn" data-expand-open data-expand-label="${esc(label)}" aria-haspopup="dialog" title="拡大" aria-label="「${esc(label)}」を拡大して読む"><span class="material-symbols-outlined" aria-hidden="true">open_in_full</span></button>`;

const cardDialogPanelCards = () => [...document.querySelectorAll('#tp-detail [data-expand]')];
/* The card in the panel the dialog shows, only when it is certain which one it is. */
function cardDialogSource(section = cardDialog.section, gate = cardDialog.gate) {
  const hits = cardDialogPanelCards().filter(el =>
    el.dataset.expand === section && (el.dataset.expandGate || null) === (gate || null));
  return hits.length === 1 ? hits[0] : null;
}
/* The gate's choices and decision dock in the panel, in panel order, other than the card itself. */
function cardDialogControls(ref, source) {
  if (!ref) return [];
  return [...document.querySelectorAll('#tp-detail [data-gate]')]
    .filter(el => el.dataset.gate === ref && el !== source && !el.contains(source) && !source?.contains(el));
}
const cardDialogPanelComment = ref => ref
  ? [...document.querySelectorAll('#tp-detail [data-gate]')].filter(el => el.dataset.gate === ref)
    .map(el => el.querySelector('.gate-comment')).find(Boolean) || null
  : null;

/* Controls that move the panel somewhere else mean nothing in a dialog about one card. */
const CARD_DIALOG_DROP = '[data-expand-open], [data-pick], [data-unpick], [data-open], [data-record], [data-judge], [data-tp-gate], [data-fetch-issue], [data-add-child]';
function cardDialogClone(el) {
  const c = el.cloneNode(true);
  c.querySelectorAll(CARD_DIALOG_DROP).forEach(x => x.remove());
  // Ids and what points at them stay in the panel: a label here must not toggle a control there.
  for (const x of [c, ...c.querySelectorAll('*')]) {
    x.removeAttribute('id'); x.removeAttribute('for');
    x.removeAttribute('aria-labelledby'); x.removeAttribute('aria-controls');
  }
  return c;
}

const cardDialogContent = () => cardDialogEl().querySelector('.card-dialog-content');
const cardDialogBox = () => cardDialogEl().querySelector('.gate-comment');

/* What the dialog keeps when the card or its gate has left the panel: the last content, readable, with
   nothing left that would answer. A comment typed stays in its box, which can no longer send it. */
function cardDialogStrip(text) {
  // Whatever comes back is drawn again, not taken as already shown.
  cardDialog.sig = '';
  cardDialogContent().querySelectorAll('[data-gate]').forEach(el => {
    el.removeAttribute('data-gate');
    el.setAttribute('data-gate-gone', '');
    el.querySelectorAll('button').forEach(x => x.remove());
    el.querySelectorAll('textarea').forEach(x => { x.readOnly = true; });
  });
  cardDialogNotice(text);
}
/* Not rewritten when unchanged: a live region would announce the same words again. */
function cardDialogNotice(text) {
  const el = cardDialogEl().querySelector('.card-dialog-gone');
  if (el.textContent !== text) el.textContent = text;
}

/* `force` redraws although a comment is being typed (the value typed is carried over); `initial` is the
   first fill, before the dialog is open. */
function cardDialogSync(force = false, initial = false) {
  const dlg = cardDialogEl();
  if (!dlg.open && !initial) return;
  // An answer on its way drops the gate from the panel before it is known to have gone: the dialog waits
  // for the outcome rather than showing the card as gone.
  if (cardDialog.answering) return;
  if (selectedTaskId !== cardDialog.subject || tp('task-panel').hidden || nav.pane !== cardDialog.pane) {
    cardDialog.quiet = true;
    if (dlg.open) dlg.close();
    return;
  }
  const source = cardDialogSource();
  const controls = cardDialogControls(cardDialog.gate, source);
  const hasGate = controls.length > 0 || (!!source && !!cardDialog.gate && source.dataset.gate === cardDialog.gate);
  if (initial) cardDialog.expectControls = hasGate;
  // Controls that showed up after the opening count too, once they have been seen.
  if (hasGate) cardDialog.expectControls = true;
  // The card is no longer in the panel: this holds while typing too.
  if (!source) {
    cardDialog.gone = true;
    cardDialog.gateGone = false;
    return cardDialogStrip(CARD_DIALOG_GONE);
  }
  // Only the gate's answer has gone: the card goes on following the panel, without the controls.
  const gateLost = cardDialog.expectControls && !hasGate;
  if (gateLost && !cardDialog.gateGone) { cardDialog.gateGone = true; cardDialogStrip(CARD_DIALOG_GATE_GONE); }
  if (!gateLost) cardDialog.gateGone = false;
  if (!force && dlg.open && document.activeElement?.matches('#card-dialog .gate-comment')) {
    cardDialog.held = true;
    return;
  }
  cardDialog.held = false;
  cardDialog.gone = false;
  cardDialogNotice(gateLost ? CARD_DIALOG_GATE_GONE : '');
  const sig = [source, ...controls].map(el => el.outerHTML).join('\u0000');
  if (sig === cardDialog.sig && !force) return;
  cardDialog.sig = sig;
  const content = cardDialogContent();
  const body = dlg.querySelector('.card-dialog-body');
  const typed = dlg.open ? cardDialogBox()?.value : cardDialogPanelComment(cardDialog.gate)?.value;
  const opened = [...content.querySelectorAll('details')].map(d => d.open);
  const scroll = body.scrollTop;
  // The controls already stripped (and their comment) stay as they are while the card is redrawn.
  const kept = gateLost ? [...content.children].slice(1).filter(el => el.hasAttribute('data-gate-gone')) : [];
  content.replaceChildren(cardDialogClone(source), ...controls.map(cardDialogClone), ...kept);
  const box = cardDialogBox();
  if (box && typed) box.value = typed;
  content.querySelectorAll('details').forEach((d, i) => { if (opened[i] !== undefined) d.open = opened[i]; });
  body.scrollTop = scroll;
}

function openCardDialog(btn) {
  const dlg = cardDialogEl();
  if (!dlg || dlg.open) return;
  const card = btn.closest('[data-expand]');
  if (!card) return;
  // The panel can have drawn the card again (or twice) since the button was seen: nothing to show then.
  if (!cardDialogSource(card.dataset.expand, card.dataset.expandGate || null)) {
    note('拡大', true, 'このカードはパネルから見つかりませんでした');
    return;
  }
  Object.assign(cardDialog, {
    subject: selectedTaskId, pane: nav.pane, section: card.dataset.expand,
    gate: card.dataset.expandGate || null, opener: btn, sig: '', held: false, quiet: false, gone: false,
    answering: false, gateGone: false, pressing: false,
    token: cardDialog.token + 1,
  });
  dlg.querySelector('#card-dialog-title').textContent = btn.dataset.expandLabel || '';
  dlg.querySelector('#card-dialog-sub').textContent = tp('tp-head').querySelector('.tp-title')?.textContent || '';
  dlg.querySelector('.card-dialog-gone').textContent = '';
  cardDialogSync(true, true);
  dlg.showModal();
  dlg.querySelector('.card-dialog-body').focus();
  cardDialog.observer ||= new MutationObserver(() => {
    if (cardDialog.pending) return;
    cardDialog.pending = true;
    queueMicrotask(() => { cardDialog.pending = false; cardDialogSync(); });
  });
  cardDialog.observer.observe(tp('tp-detail'), { childList: true, subtree: true });
  cardDialog.observer.observe(tp('task-panel'), { attributes: true, attributeFilter: ['hidden'] });
}
function closeCardDialog() {
  const dlg = cardDialogEl();
  if (dlg?.open) dlg.close();
}

/* Where focus goes when the dialog closes: the button that opened it, else the same card's button
   if the panel drew it again, else something of the panel that is shown. */
function cardDialogFocusTarget() {
  const shown = el => el && el.isConnected && el.getClientRects().length > 0;
  const again = cardDialogPanelCards().filter(el =>
    el.dataset.expand === cardDialog.section && (el.dataset.expandGate || null) === (cardDialog.gate || null))
    .map(el => el.querySelector('[data-expand-open]'))[0];
  return [cardDialog.opener, again, document.querySelector('#tp-tabs [aria-selected="true"]'),
    document.querySelector('#task-panel [data-tp-close]')].find(shown) || null;
}

tp('task-panel').addEventListener('click', e => {
  const b = e.target.closest('[data-expand-open]');
  if (b) openCardDialog(b);
});

/* A button of the gate's controls: a sent answer closes the dialog, a failed one keeps it and what was typed. */
async function cardDialogAnswer(b) {
  // A second click while the first answer is on its way: that one decides what happens, and `answering` is its own.
  if (cardDialog.answering) return;
  const token = cardDialog.token;
  let sent;
  cardDialog.answering = true;
  try { sent = await decideAct(b); } finally { if (token === cardDialog.token) cardDialog.answering = false; }
  // Only the opening the click was made in: another may have been opened while the answer was on its way.
  if (token !== cardDialog.token || !cardDialogEl().open) return;
  if (sent) closeCardDialog();
  else {
    // What a failed answer leaves: a redraw held back, a gate that has vanished. An identical redraw is skipped,
    // so the button that was pressed keeps its focus.
    cardDialog.held = false;
    cardDialogSync();
  }
}

cardDialogEl().addEventListener('click', async e => {
  const hit = sel => e.target.closest(sel);
  let b;
  if (hit('[data-card-dialog-close]')) return closeCardDialog();
  // Leaving for the terminal: the dialog goes first, and focus is not given back to the panel.
  if ((b = hit('[data-gate] [data-act="talk"]'))) {
    cardDialog.quiet = true; closeCardDialog();
    return void decideAct(b);
  }
  if ((b = hit('[data-focus]'))) {
    cardDialog.quiet = true; closeCardDialog();
    return void worktreeAct('focus', b.dataset.focus);
  }
  if ((b = hit('[data-gate] [data-act], [data-gate] .pick[data-choice]'))) return cardDialogAnswer(b);
  if (parkClick(e)) return;
  if ((b = hit('[data-ide]'))) worktreeAct('ide', b.dataset.ide);
});
cardDialogEl().addEventListener('change', e => {
  const box = e.target.closest?.('input[data-manual-gate]');
  if (!box) return;
  // The panel's checkbox is the one `manualChecks` follows (decide.js); this one sets it and the redraw keeps it.
  for (const p of document.querySelectorAll('#tp-detail input[data-manual-gate]')) {
    if (p.dataset.manualGate === box.dataset.manualGate && p.dataset.manualIndex === box.dataset.manualIndex) {
      p.checked = box.checked;
      p.dispatchEvent(new Event('change', { bubbles: true }));
    }
  }
});
cardDialogEl().addEventListener('focusout', e => {
  if (!cardDialog.held || !e.target.matches('.gate-comment')) return;
  // A button of the gate's controls is about to be clicked, and the click resyncs or closes. Anywhere else
  // (the text, the next control) the held redraw is flushed now.
  if (e.relatedTarget?.closest?.('[data-gate] button') && cardDialogEl().contains(e.relatedTarget)) return;
  // Some browsers give a clicked button no focus, so relatedTarget says nothing: a press in progress waits.
  if (cardDialog.pressing) return;
  setTimeout(() => {
    if (cardDialog.pressing || !cardDialog.held || document.activeElement?.matches('#card-dialog .gate-comment')) return;
    cardDialogSync();
  });
});
/* A press on a button of the gate's controls: the held redraw waits until the click has been handled, or the
   press is cancelled, so the button is not replaced between mousedown and mouseup. */
function cardDialogPressEnd() {
  if (!cardDialog.pressing) return;
  setTimeout(() => {
    cardDialog.pressing = false;
    if (cardDialog.held && !document.activeElement?.matches('#card-dialog .gate-comment')) cardDialogSync();
  });
}
cardDialogEl().addEventListener('pointerdown', e => {
  // A press that never ended (released outside, no click) is over once another begins.
  cardDialog.pressing = !!e.target.closest?.('[data-gate] button');
});
// Released outside the dialog, or a right-click: no click will come to end the press.
document.addEventListener('pointerup', e => { if (!cardDialogEl().contains(e.target)) cardDialogPressEnd(); }, true);
document.addEventListener('contextmenu', cardDialogPressEnd, true);
cardDialogEl().addEventListener('click', cardDialogPressEnd);
cardDialogEl().addEventListener('pointercancel', cardDialogPressEnd);
cardDialogEl().addEventListener('close', () => {
  cardDialog.observer?.disconnect();
  const typed = cardDialogBox()?.value;
  const panelBox = cardDialogPanelComment(cardDialog.gate);
  if (panelBox && typed != null && selectedTaskId === cardDialog.subject) panelBox.value = typed;
  if (!cardDialog.quiet) cardDialogFocusTarget()?.focus();
  cardDialog.subject = null; cardDialog.opener = null; cardDialog.sig = ''; cardDialog.held = false;
  cardDialog.quiet = false; cardDialog.gone = false; cardDialog.expectControls = false; cardDialog.pressing = false; cardDialog.answering = false; cardDialog.gateGone = false;
});
