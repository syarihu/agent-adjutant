/* 「拡大」: a large dialog to read in. For a task it holds the whole task in one scroll, drawn from the data
   with the panel's own card renderers (task-view.js, decide.js), under a left index of what is in it, and a right
   column to answer the waiting gates in, one at a time; answering goes on to the next waiting gate. For what has
   no task (a hub, a session, a gate of its own) it holds one card alone, copied from the panel's DOM, which it
   follows while it is open and which brings its gate's answer controls along; answering closes the dialog. */

const cardDialog = {
  subject: null, pane: null, section: null, gate: null, opener: null,
  sig: '', held: false, quiet: false, gone: false, pending: false, observer: null,
  expectControls: false, token: 0, pressing: false, answering: false, gateGone: false,
  // The whole task: `mode` is 'task' then, `target` the card to bring into view once it is drawn, `dock` the ref of
  // the gate the answer column shows, `dockSig` what the column was drawn from, `groups` what was drawn last, `active`
  // the group the index marks, `answered` the gates answered in this opening (ref -> a copy, until the history has
  // the record), `answeredHere` their refs for good (the history's record must not make one look answered elsewhere), `noMore` that an answer left nothing waiting.
  // What changed while it is open (#630): `cardSigs` the time-free markup of each card as last drawn, `updated` the cards
  // marked 「更新」 until they have been seen, `updatedDocks` the gates whose switch is marked the same way, `seenTimers`
  // the cards seen and about to be let go, `updateTimer`, `updateText` and `updateChanges` the notice under the head, `waitingRefs` the gates
  // that waited at the last sync, `rebase` that the next content sync only takes over what it draws, marking nothing.
  mode: 'card', target: null, targetAt: 0, pinned: null, pinnedAt: 0, dock: null, dockSig: '', groups: [], gateIds: {}, active: null, spy: false,
  answered: new Map(), answeredHere: new Set(), noMore: false,
  cardSigs: new Map(), updated: new Set(), updatedDocks: new Set(), seenTimers: new Map(), updateTimer: 0, updateText: '', updateChanges: null, waitingRefs: [], rebase: false,
};
const CARD_DIALOG_GONE = 'パネルからこのカードがなくなりました（最後に表示した内容です）';
const CARD_DIALOG_GATE_GONE = 'この gate への答えはパネルから消えました（最後に表示した内容です）';
const CARD_DIALOG_TASK_GONE = 'パネルからこのタスクがなくなりました（最後に表示した内容です）';
const CARD_DIALOG_NONE_LEFT = '判断待ちはもうありません';
const cardDialogDrafts = new Map(); // gate ref -> a comment typed in the dialog, kept past a close: the panel has a box for one gate only
const cardDialogEl = () => document.getElementById('card-dialog');
const cardDialogDockEl = () => cardDialogEl().querySelector('.card-dialog-dock');

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
  cardDialog.sig = ''; cardDialog.dockSig = '';
  cardDialogEl().querySelectorAll('[data-gate]').forEach(cardDialogStripEl);
  cardDialogNotice(text);
}
function cardDialogStripEl(el) {
  el.removeAttribute('data-gate');
  el.setAttribute('data-gate-gone', '');
  el.querySelectorAll('button').forEach(x => x.remove());
  el.querySelectorAll('textarea').forEach(x => { x.readOnly = true; });
}
/* Not rewritten when unchanged: a live region would announce the same words again. */
function cardDialogNotice(text) {
  const el = cardDialogEl().querySelector('.card-dialog-gone');
  if (el.textContent !== text) el.textContent = text;
}

/* `force` redraws although a comment is being typed (the value typed is carried over); `initial` is the
   first fill, before the dialog is open. */
function cardDialogSync(force = false, initial = false) {
  if (cardDialog.mode === 'task') return cardDialogTaskSync(force, initial);
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
  const task = cardDialogTask();
  if (task) return openCardDialogTask(task, btn);
  const card = btn.closest('[data-expand]');
  if (!card) return;
  // The panel can have drawn the card again (or twice) since the button was seen: nothing to show then.
  if (!cardDialogSource(card.dataset.expand, card.dataset.expandGate || null)) {
    note('拡大', true, 'このカードはパネルから見つかりませんでした');
    return;
  }
  Object.assign(cardDialog, {
    mode: 'card', subject: selectedTaskId, pane: nav.pane, section: card.dataset.expand,
    gate: card.dataset.expandGate || null, opener: btn, sig: '', held: false, quiet: false, gone: false,
    answering: false, gateGone: false, pressing: false, target: null, dock: null, dockSig: '', groups: [],
    answered: new Map(), answeredHere: new Set(), noMore: false, token: cardDialog.token + 1,
  });
  cardDialogShell('card', btn.dataset.expandLabel || '', tp('tp-head').querySelector('.tp-title')?.textContent || '');
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
/* What the dialog has before anything is drawn in it. */
function cardDialogShell(mode, title, sub) {
  const dlg = cardDialogEl();
  dlg.querySelector('#card-dialog-title').textContent = title;
  dlg.querySelector('#card-dialog-sub').textContent = sub;
  dlg.querySelector('.card-dialog-gone').textContent = '';
  cardDialogResetUpdates();
  // Nothing of an earlier opening may be read back: what is kept across redraws is read from this DOM.
  const dock = dlg.querySelector('.card-dialog-dock');
  dock.replaceChildren();
  dock.hidden = true;
  const content = dlg.querySelector('.card-dialog-content');
  content.replaceChildren();
  content.dataset.mode = mode;
  const index = dlg.querySelector('.card-dialog-index');
  index.hidden = mode !== 'task';
  index.replaceChildren();
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
  // Opened from the head's button, which no card stands for.
  const head = cardDialog.section ? null : document.querySelector('#task-panel [data-expand-task]');
  return [cardDialog.opener, again, head, document.querySelector('#tp-tabs [aria-selected="true"]'),
    document.querySelector('#task-panel [data-tp-close]')].find(shown) || null;
}

tp('task-panel').addEventListener('click', e => {
  const b = e.target.closest('[data-expand-open], [data-expand-task]');
  if (b) openCardDialog(b);
});

/* A button of the gate's controls. In a task a sent answer goes on to the next waiting gate; for a card alone it
   closes the dialog. A failed one keeps the dialog and what was typed. */
async function cardDialogAnswer(b) {
  // A second click while the first answer is on its way: that one decides what happens, and `answering` is its own.
  if (cardDialog.answering) return;
  const token = cardDialog.token;
  // What the answer was for, taken before the gate leaves the panel's data.
  const ref = b.closest?.('[data-gate]')?.dataset.gate;
  const act = b.matches?.('.pick[data-choice]') ? 'choice' : b.dataset?.act;
  const g = cardDialog.mode === 'task' && ref ? gateByRef(ref) : null;
  const comment = (ref && commentBox(ref)?.value.trim()) || '';
  let sent;
  cardDialog.answering = true;
  try { sent = await decideAct(b); } finally { if (token === cardDialog.token) cardDialog.answering = false; }
  // Only the opening the click was made in: another may have been opened while the answer was on its way.
  if (token !== cardDialog.token || !cardDialogEl().open) return;
  if (!sent) {
    // What a failed answer leaves: a redraw held back, a gate that has vanished. An identical redraw is skipped,
    // so the button that was pressed keeps its focus.
    cardDialog.held = false;
    cardDialogSync();
  } else if (cardDialog.mode !== 'task') closeCardDialog();
  else if (g && g.wait !== false) cardDialogAfterAnswer(ref, g, act, b.dataset?.choice, comment);
  else cardDialogSync();
}
/* The answered gate is kept here until the history has its record, and the column goes on to the next waiting gate. */
function cardDialogAfterAnswer(ref, g, act, choice, comment) {
  cardDialog.answeredHere.add(ref);
  cardDialogDrafts.delete(ref);
  // What the answer itself changes (the history, the plan, the details) is not news to the person who gave it.
  cardDialog.rebase = true;
  cardDialog.answered.set(ref, {
    ...g, decision: act === 'close' ? 'closed' : act, choice: choice ?? g.choice, comment, answeredAt: secsStamp(Date.now() / 1000),
  });
  const token = cardDialog.token;
  const task = taskById(cardDialog.subject);
  const all = task ? cardDialogWithAnswered(gatesOf(task), cardDialog.answered) : [];
  const waiting = cardDialogWaiting(all);
  // The one after the answered gate in the order they are read, else the first from the top.
  const next = all.slice(all.findIndex(g => gateRef(g) === ref) + 1).find(g => waiting.includes(g)) || waiting[0] || null;
  // Before the sync, so that the sync does not take this answer, or its comment, for a gate answered elsewhere.
  cardDialog.dock = next ? gateRef(next) : null;
  cardDialog.noMore = !next;
  cardDialog.held = false;
  // Forced: a comment typed for another gate must not hold the redraw back while this dock is still on show.
  cardDialogSync(true);
  // The sync can close the dialog (the task left, the panel moved on): nothing is left to move in.
  if (!cardDialogEl().open || cardDialog.token !== token) return;
  if (next) {
    cardDialogGoGate(gateRef(next));
    // Keyboard on, so that the gates can be answered in a row.
    [...cardDialogDockEl().querySelectorAll('[data-cd-dock-for]')].find(el => el.dataset.cdDockFor === gateRef(next))
      ?.querySelector('.gate-comment')?.focus({ preventScroll: true });
  }
  // The column is gone with the last gate, and so is the focus that was in it.
  else cardDialogEl().querySelector('.card-dialog-body').focus({ preventScroll: true });
}

/* A button of a gate's controls. A pick answers the gate it is in: the column shows that gate first, so the comment
   sent is the one shown for it; while another answer is on its way a pick does nothing at all. */
function cardDialogGateButton(b) {
  if (cardDialog.answering) return;
  if (cardDialog.mode === 'task' && b.matches('.pick[data-choice]')) cardDialogSetDock(b.closest('[data-gate]').dataset.gate, true);
  return cardDialogAnswer(b);
}

cardDialogEl().addEventListener('click', async e => {
  const hit = sel => e.target.closest(sel);
  let b;
  if (hit('[data-card-dialog-close]')) return closeCardDialog();
  // The index and the timeline move inside the dialog; nothing is sent.
  if ((b = hit('[data-cd-go-group]'))) return cardDialogGo(b.dataset.cdGoCard || null, b.dataset.cdGoGroup, false);
  if ((b = hit('[data-cd-goto]'))) return cardDialogGoGate(b.dataset.cdGoto);
  if ((b = hit('[data-cd-dock]'))) return cardDialogSetDock(b.dataset.cdDock, true);
  if ((b = hit('[data-cd-see]'))) return cardDialogSeeNotice(b.dataset.cdSee);
  // Leaving for the terminal: the dialog goes first, and focus is not given back to the panel.
  if ((b = hit('[data-gate] [data-act="talk"]'))) {
    cardDialog.quiet = true; closeCardDialog();
    return void decideAct(b);
  }
  if ((b = hit('[data-focus]'))) {
    cardDialog.quiet = true; closeCardDialog();
    return void worktreeAct('focus', b.dataset.focus);
  }
  if ((b = hit('[data-gate] [data-act], [data-gate] .pick[data-choice]'))) return cardDialogGateButton(b);
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
/* The buttons that act on a gate: its controls, and the column's own (the switch and 「この gate へ」). */
const CARD_DIALOG_BUTTON = '[data-gate] button, .card-dialog-dock button';
cardDialogEl().addEventListener('focusout', e => {
  if (!cardDialog.held || !e.target.matches('.gate-comment')) return;
  // A button of the gate's controls is about to be clicked, and the click resyncs or closes. Anywhere else
  // (the text, the next control) the held redraw is flushed now.
  if (e.relatedTarget?.closest?.(CARD_DIALOG_BUTTON) && cardDialogEl().contains(e.relatedTarget)) return;
  // Some browsers give a clicked button no focus, so relatedTarget says nothing: a press in progress waits.
  if (cardDialog.pressing) return;
  setTimeout(() => {
    if (cardDialog.pressing || !cardDialog.held || document.activeElement?.matches('#card-dialog .gate-comment')) return;
    cardDialogSync();
  });
});
/* A press on one of those buttons: the held redraw waits until the click has been handled, or the
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
  cardDialog.pressing = !!e.target.closest?.(CARD_DIALOG_BUTTON);
});
// Released outside the dialog, or a right-click: no click will come to end the press.
document.addEventListener('pointerup', e => { if (!cardDialogEl().contains(e.target)) cardDialogPressEnd(); }, true);
document.addEventListener('contextmenu', cardDialogPressEnd, true);
cardDialogEl().addEventListener('click', cardDialogPressEnd);
cardDialogEl().addEventListener('pointercancel', cardDialogPressEnd);
cardDialogEl().addEventListener('close', () => {
  cardDialog.observer?.disconnect();
  // What was typed goes back to the box of the same gate in the panel, and to no other.
  cardDialogEl().querySelectorAll('[data-gate]').forEach(el => {
    const typed = el.querySelector('.gate-comment')?.value;
    const panelBox = selectedTaskId === cardDialog.subject ? cardDialogPanelComment(el.dataset.gate) : null;
    if (panelBox && typed != null) panelBox.value = typed;
    // A gate the panel has no box for keeps its draft here, for the next opening.
    if (cardDialog.mode === 'task' && typed != null) typed ? cardDialogDrafts.set(el.dataset.gate, typed) : cardDialogDrafts.delete(el.dataset.gate);
  });
  if (!cardDialog.quiet) cardDialogFocusTarget()?.focus();
  cardDialog.subject = null; cardDialog.opener = null; cardDialog.sig = ''; cardDialog.held = false;
  cardDialog.quiet = false; cardDialog.gone = false; cardDialog.expectControls = false; cardDialog.pressing = false; cardDialog.answering = false; cardDialog.gateGone = false;
  cardDialog.target = null; cardDialog.pinned = null; cardDialog.dock = null; cardDialog.groups = []; cardDialog.gateIds = {}; cardDialog.active = null;
  cardDialog.dockSig = ''; cardDialog.answered = new Map(); cardDialog.answeredHere = new Set(); cardDialog.noMore = false;
  // What was drawn goes with the opening, and so does its notice.
  cardDialogContent().replaceChildren();
  cardDialogEl().querySelector('.card-dialog-index').replaceChildren();
  const dock = cardDialogDockEl();
  dock.replaceChildren();
  dock.hidden = true;
  cardDialogEl().querySelector('.card-dialog-gone').textContent = '';
  cardDialogResetUpdates();
});

/* ---- The whole task ---- */

/* The task the panel shows, if it shows one; what has none keeps the single-card path. */
const cardDialogTask = () => selectedTaskId && !isHubRef(selectedTaskId) ? taskById(selectedTaskId) : null;

/* The waiting gates of the task, oldest first, with their open time as `gatesOf` orders them. */
const cardDialogWaiting = all => all.filter(g => g.wait !== false && isWaiting(g));

/* The heading of an answered gate's block: what it was, how and when it was answered, and the answer. */
function cardDialogAnsweredHeadHtml(g) {
  const bad = ['changes', 'reject'].includes(g.decision);
  const picked = g.choice ? (g.choices || []).find(c => c.id === g.choice) : null;
  const answer = [picked ? `選んだ案: ${picked.label}` : '', g.comment || ''].filter(Boolean).join(' — ');
  const why = stopWhy(g);
  return `<div class="cd-gate-head"><div class="cd-gate-title-row">` +
    `<h4 class="cd-gate-title">${esc(`【${kindOf(g.kind)[0]}】${g.title}`)}</h4>` +
    `<span class="cd-badge${bad ? ' cd-bad' : ''}">${esc(DECISION[g.decision] || '回答済み')}</span>` +
    (g.answeredAt ? `<span class="cd-gate-when" title="${esc(when(g.answeredAt))}">${esc(ago(g.answeredAt))}に回答</span>` : '') +
    `</div>` + (answer ? `<p class="cd-gate-answer">${esc(answer)}</p>` : '') +
    (why.length ? `<p class="cd-gate-why${stopBad(g) ? ' cd-bad' : ''}">止めた理由: ${esc(why.join(' / '))}</p>` : '') + `</div>`;
}

/* What the dialog holds, as groups of cards, in the order they are read: what waits, the review, the diff, the
   plan, what was answered, 経過, 詳細. A group or card with nothing to say is left out, and so are the panel's 記録 list
   and 工程. The choices of every waiting gate can be picked here.
   `{ key, label, badge, gate?, cards: [{ key, label, badge, wide, html, expand: [section, gateRef] | null, gate? }] }`;
   `gate` is the ref of the gate a waiting group is for, and of the gate a card of it, or an answered gate's card, is about. */
function cardDialogTaskGroups(task, all) {
  const card = (key, label, html, o = {}) => html ? { key, label, badge: o.badge || '', wide: !!o.wide, html, expand: o.expand || null, ...(o.gate ? { gate: o.gate } : {}) } : null;
  const at = (section, g) => [section, g ? gateRef(g) : null];
  const decidedHtml = g => g.decided ? `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<h3>決定事項</h3><div class="body">${md(g.decided)}</div></div>` : '';
  const runHtml = g => g.run ? `<div class="panel"${expandAttrs('run', g)}>${expandBtnHtml('動かし方')}<h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>` : '';
  const group = (key, label, cards, badge = '', gate = null) => {
    const kept = cards.filter(Boolean);
    return kept.length ? { key, label, badge, ...(gate ? { gate } : {}), cards: kept } : null;
  };
  // The review, the diff and the plan are the latest (or the waiting) gate of each kind, whatever chip the panel has
  // picked; the earlier rounds are in the answered gates.
  const gR = gateForTab(task, 'review', all, {});
  const reviewCards = (g, k) => [
    card(`${k}:rounds`, 'セルフレビュー', roundsCardHtml(g), { expand: at('rounds', g) }),
    card(`${k}:findings`, '指摘', findingsCardHtml(g), { wide: true, expand: at('findings', g) }),
  ];
  const diffCards = (g, k) => [
    card(`${k}:files`, 'ファイル', filesTableHtml(g), { wide: true }),
    card(`${k}:diff`, '差分', diffCardHtml(g), { wide: true, expand: at('diff', g) }),
  ];

  // 1. a group for each gate waiting for an answer
  const waiting = cardDialogWaiting(all).map(g => {
    const ref = gateRef(g);
    const k = part => `gate:${ref}:${part}`;
    const cards = [
      card(k('head'), '止まっている理由', gateWaitHeadHtml(g, { focus: false }), { wide: true, expand: at('wait', g) }),
      card(k('facts'), '事実', factsCardHtml(g), { expand: at('facts', g) }),
      card(k('focus'), '確認してほしい点', focusCardHtml(g), { expand: at('focus', g) }),
      card(k('unsure'), '迷っていること', unsureCardHtml(g), { expand: at('unsure', g) }),
      card(k('report'), '報告', reportCardHtml(g), { expand: at('report', g) }),
      card(k('choices'), '選択肢', choicesHtml(g, true), { wide: true, expand: at('choices', g) }),
    ];
    if (g.kind === 'verify') {
      cards.push(card(k('commands'), 'Verify 実行結果', commandsCardHtml(g), { expand: at('commands', g) }),
        card(k('manual'), '人が見る確認項目', manualCardHtml(g), { expand: at('manual', g) }),
        card(k('run'), '動かし方', runHtml(g), { expand: at('run', g) }));
    }
    // The review and the diff of another waiting gate of the kind: group 2 and 3 hold only one.
    if (g.kind === 'diff' && g !== gR) cards.push(...reviewCards(g, `gate:${ref}`), ...diffCards(g, `gate:${ref}`));
    if (g.kind !== 'plan') cards.push(card(k('decided'), '決定事項', decidedHtml(g), { expand: at('decided', g) }));
    return group(`waiting:${ref}`, `【${kindOf(g.kind)[0]}】${g.title}`, cards.map(c => c && { ...c, gate: ref }), '判断待ち', ref);
  });

  // 2 and 3. what the review tab shows: a record of the review can be all there is
  const review = gR ? [
    ...reviewCards(gR, 'review'),
    // A record has no group of its own below, so what it carries is here.
    ...(gR.wait === false ? [
      card('review:facts', '事実', factsCardHtml(gR), { expand: at('facts', gR) }),
      card('review:focus', '確認してほしい点', focusCardHtml(gR), { expand: at('focus', gR) }),
      card('review:unsure', '迷っていること', unsureCardHtml(gR), { expand: at('unsure', gR) }),
      card('review:report', '報告', reportCardHtml(gR), { expand: at('report', gR) }),
    ] : []),
  ] : [];

  // 4. the plan, and what it answers
  const plan = gateShownIn(task, 'overview', all, {});
  const planCards = [
    card('plan:issue', 'Issue の本文', issueBodyCardHtml(task), { wide: true, expand: ['issue', null] }),
    card('plan:problem', '問題', problemCardHtml(task, plan), { expand: ['problem', plan?.problem ? gateRef(plan) : null] }),
    // The problem shows the request itself while the plan has none of its own.
    card('plan:request', '依頼内容', task.body && plan?.problem
      ? `<div class="panel"${expandAttrs('request')}>${expandBtnHtml('依頼内容')}<h3>依頼内容</h3><div class="body">${md(task.body)}</div></div>` : '', { expand: ['request', null] }),
    card('plan:goal', 'ゴール', goalCardHtml(task, plan), { expand: ['goal', plan?.goal ? gateRef(plan) : null] }),
    card('plan:head', '計画', planHeadCardHtml(plan, all.filter(g => g.kind === 'plan'), { chips: false, focus: false }), { expand: at('plan', plan) }),
    card('plan:decided', '決定事項', plan ? decidedHtml(plan) : '', { expand: at('decided', plan) }),
  ];

  // 5. the gates answered: one wide block for each, a heading over its parts
  const answered = all.filter(g => !isWaiting(g) && g.wait !== false).map(g => {
    const part = (html, wide = false) => html ? `<div class="cd-part${wide ? ' cd-wide' : ''}">${html}</div>` : '';
    const parts = [
      part(factsCardHtml(g)), part(focusCardHtml(g)), part(unsureCardHtml(g)), part(reportCardHtml(g)),
      part(choicesHtml(g, false), true),
      ...(g.kind === 'verify' ? [part(commandsCardHtml(g), true), part(manualCardHtml(g)), part(runHtml(g))]
        : g.kind === 'diff' && g !== gR ? [part(roundsCardHtml(g), true), part(findingsCardHtml(g), true)] : []),
      // The plan shown in the plan group has its 決定事項 there.
      part(g === plan ? '' : decidedHtml(g)),
    ].join('');
    const html = `<div class="cd-gate">${cardDialogAnsweredHeadHtml(g)}${parts ? `<div class="cd-gate-parts">${parts}</div>` : ''}</div>`;
    return card(`gate:${gateRef(g)}`, `【${kindOf(g.kind)[0]}】${g.title}`, html, { badge: DECISION[g.decision] || '回答済み', wide: true, expand: [null, gateRef(g)], gate: gateRef(g) });
  });

  // 7. the agent's last words, and the rest of what is known of the task
  const sess = sessionOfTask(task);
  const last = sess?.present && sess.agentSession ? lastMessageHtml(sess.agentSession) : '';

  return [
    ...waiting,
    group('review', 'セルフレビューと指摘', review),
    group('diff', '差分', gR ? diffCards(gR, 'review') : []),
    group('plan', '依頼と計画', planCards),
    group('answered', '回答済み', answered),
    group('history', '経過', [card('history:timeline', '経過', timelineHtml(task, all), { wide: true })]),
    group('details', '詳細', [
      card('details:lastmsg', '最後のメッセージ', last, { expand: ['lastmsg', null] }),
      card('details:kv', '詳細', detailsKvHtml(task, { refs: true, park: true }), { wide: true }),
    ]),
  ].filter(Boolean);
}

/* The card a button of the panel (or the timeline) means: the one of that section and gate, else any of that
   gate, else one of that section; its key, or null (the top). */
function cardDialogResolveTarget(groups, target) {
  if (!target) return null;
  const cards = groups.flatMap(g => g.cards);
  const found = cards.find(c => c.expand && c.expand[0] === target.section && (c.expand[1] || null) === (target.gate || null))
    || (target.gate && cards.find(c => c.expand && c.expand[1] === target.gate))
    // A card of one gate never lands on another gate's card of the same kind.
    || (!target.gate && target.section && cards.find(c => c.expand && c.expand[0] === target.section));
  return found ? found.key : null;
}

/* The gate the answer column starts on: the opened card's gate if it waits, else the first that waits. */
function cardDialogDockGate(all, ref) {
  const waiting = cardDialogWaiting(all);
  return (ref && waiting.find(g => gateRef(g) === ref)) || waiting[0] || null;
}

/* The index and the content, as markup. Every label and key is escaped; a key is only ever an attribute value. */
function cardDialogTaskHtml(groups) {
  const entry = (g, c) => `<li><button type="button" data-cd-go-group="${esc(g.key)}"${c ? ` data-cd-go-card="${esc(c.key)}"` : ''}>` +
    `<span>${esc((c || g).label)}</span>${(c || g).badge ? `<span class="cd-badge">${esc((c || g).badge)}</span>` : ''}</button>`;
  const index = `<ol>` + groups.map(g => entry(g) + `<ol>${g.cards.map(c => entry(g, c) + '</li>').join('')}</ol></li>`).join('') + `</ol>`;
  const content = groups.map(g => `<section class="cd-group" data-cd-group="${esc(g.key)}" aria-label="${esc(g.label)}" tabindex="-1">` +
    `<h3 class="cd-group-title"><span>${esc(g.label)}</span>${g.badge ? `<span class="cd-badge">${esc(g.badge)}</span>` : ''}</h3>` +
    `<div class="cd-grid">${g.cards.map(c => `<div class="cd-card${c.wide ? ' cd-wide' : ''}" data-cd-card="${esc(c.key)}" tabindex="-1">${c.html}</div>`).join('')}</div></section>`).join('');
  return { index, content };
}

/* The answer column, as markup: a switch when more than one gate waits, and a dock for each waiting gate. Which one is
   shown is set apart from the markup (`cardDialogShowDock`), so a switch redraws nothing. */
function cardDialogDockHtml(waiting) {
  const name = g => `【${kindOf(g.kind)[0]}】${g.title}`;
  const pick = waiting.length > 1 ? `<div class="cd-dock-switch" role="group" aria-label="答える gate">` +
    waiting.map(g => `<button type="button" data-cd-dock="${esc(gateRef(g))}" aria-pressed="false" title="${esc(name(g))}">${esc(name(g))}</button>`).join('') + `</div>` : '';
  return pick + waiting.map(g => `<section class="cd-dock" data-cd-dock-for="${esc(gateRef(g))}" role="group" aria-label="${esc(`${name(g)} に答える`)}" hidden>` +
    `<div class="cd-dock-head"><p class="cd-dock-label">${esc(name(g))} に答える</p><button type="button" class="btn-m3-text" data-cd-goto="${esc(gateRef(g))}">この gate へ</button></div>` +
    `${decideHtml(g)}</section>`).join('');
}

/* Shows the dock of `cardDialog.dock` and no other, and marks it in the switch. A dock kept after its gate was
   answered elsewhere has no `data-cd-dock-for` and stays as it is. */
function cardDialogShowDock() {
  const col = cardDialogDockEl();
  col.querySelectorAll('[data-cd-dock-for]').forEach(el => { el.hidden = el.dataset.cdDockFor !== cardDialog.dock; });
  col.querySelectorAll('[data-cd-dock]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.cdDock === cardDialog.dock)));
  col.hidden = !col.querySelector('.cd-dock');
  // A gate that joined while it was out of sight is seen now.
  if (cardDialog.updatedDocks.delete(cardDialog.dock)) cardDialogMarkUpdated();
}
/* Switches the column to the gate `ref`, if it has a dock for it. Each dock keeps its own comment box, so what was
   typed for one gate stays when the column shows another. `explicit` is a click of the person's: focus that was in the
   dock that goes away follows into the new one. Scrolling does not do that, or a textarea would take the keyboard. */
function cardDialogSetDock(ref, explicit = false) {
  const col = cardDialogDockEl();
  const next = [...col.querySelectorAll('[data-cd-dock-for]')].find(el => el.dataset.cdDockFor === ref);
  if (!next) return;
  const was = document.activeElement?.closest?.('[data-cd-dock-for]');
  cardDialog.dock = ref;
  cardDialogShowDock();
  // Focus in the dock that has just been hidden would be lost.
  if (!was || !was.hidden) return;
  if (explicit) next.querySelector('.gate-comment')?.focus({ preventScroll: true });
  else cardDialogEl().querySelector('.card-dialog-body').focus({ preventScroll: true });
}

/* The answered gates of this opening, in place among the task's gates until the history has their records. */
function cardDialogWithAnswered(all, answered) {
  if (!answered.size) return all;
  const out = [...all];
  for (const [ref, copy] of [...answered]) {
    const i = out.findIndex(g => gateRef(g) === ref);
    if (i >= 0 && out[i].decision) answered.delete(ref);
    else if (i < 0) out.push(copy);
    else if (!isWaiting(out[i])) out[i] = copy;
  }
  return out.sort((a, b) => (a.openedAt || '').localeCompare(b.openedAt || '') || claimSeq(a) - claimSeq(b));
}
/* What the notice under the head says: the docks kept after their gate was answered elsewhere, or that nothing waits
   any more after an answer. */
function cardDialogNoticeText(kept, waiting) {
  if (kept.length) return CARD_DIALOG_GATE_GONE;
  return cardDialog.noMore && !waiting.length ? CARD_DIALOG_NONE_LEFT : '';
}

/* The content as nodes, cleaned as the single card is: what moves the panel somewhere else is gone (the timeline's
   links to a gate become jumps inside the dialog, or plain text when that gate has no card), and ids and what
   points at them stay in the panel. */
function cardDialogFill(html, groups, refOfId) {
  const t = document.createElement('template');
  t.innerHTML = html;
  const root = t.content;
  root.querySelectorAll('[data-open]').forEach(b => {
    const ref = refOfId(b.dataset.open);
    if (ref && cardDialogResolveTarget(groups, { section: null, gate: ref })) {
      b.removeAttribute('data-open');
      b.setAttribute('data-cd-goto', ref);
    } else b.replaceWith(document.createTextNode(b.textContent));
  });
  root.querySelectorAll(CARD_DIALOG_DROP).forEach(x => x.remove());
  root.querySelectorAll('[id], [for], [aria-labelledby], [aria-controls]').forEach(x => {
    x.removeAttribute('id'); x.removeAttribute('for');
    x.removeAttribute('aria-labelledby'); x.removeAttribute('aria-controls');
  });
  return root;
}

const cardDialogCardEl = key => [...cardDialogContent().querySelectorAll('[data-cd-card]')].find(el => el.dataset.cdCard === key) || null;
const cardDialogGroupEl = key => [...cardDialogContent().querySelectorAll('[data-cd-group]')].find(el => el.dataset.cdGroup === key) || null;

/* Brings a card (or a group) to the top and puts focus on it; `flash` marks it for a moment. */
function cardDialogGo(cardKey, groupKey, flash) {
  const el = cardKey ? cardDialogCardEl(cardKey) : cardDialogGroupEl(groupKey);
  if (!el) return false;
  el.scrollIntoView({ block: 'start' });
  el.focus({ preventScroll: true });
  // The person has moved on by hand: a target still waiting for its data is not theirs any more, and the
  // index marks where they went until they scroll.
  cardDialog.target = null;
  cardDialog.pinned = groupKey || cardDialog.groups.find(g => g.cards.some(c => c.key === cardKey))?.key || null;
  cardDialog.pinnedAt = cardDialogEl().querySelector('.card-dialog-body').scrollTop;
  if (flash) {
    el.classList.remove('cd-flash');
    void el.offsetWidth;
    el.classList.add('cd-flash');
    // Under reduced motion nothing animates, so no animationend comes.
    setTimeout(() => el.classList.remove('cd-flash'), 1600);
  }
  cardDialogSpy();
  return true;
}
function cardDialogGoGate(ref) {
  const key = cardDialogResolveTarget(cardDialog.groups, { section: null, gate: ref });
  if (key) cardDialogGo(key, null, true);
}
cardDialogContent().addEventListener('animationend', e => e.target.classList?.remove('cd-flash'));

/* The group the index marks: the last one whose top has reached the top of the scroll. */
function cardDialogActiveGroup(tops, scrollTop, offset = 24) {
  let active = tops.length ? 0 : -1;
  tops.forEach((t, i) => { if (t <= scrollTop + offset) active = i; });
  return active;
}
/* The column follows the reading when it comes to another waiting gate's group. A switch by hand holds while the
   person stays in the group they are reading, and nothing moves under their hands while they type in the column (a box with text in it) or answer. */
function cardDialogFollowDock(groupKey) {
  const ref = cardDialog.groups.find(g => g.key === groupKey)?.gate;
  // While the card the dialog opened at is still to be found, the first group seen is not where the person is reading.
  if (!ref || ref === cardDialog.dock || cardDialog.answering || cardDialog.target || (document.activeElement?.matches?.('.card-dialog-dock .gate-comment') && document.activeElement.value)) return;
  cardDialogSetDock(ref);
}
function cardDialogSpy() {
  if (cardDialog.mode !== 'task') return;
  const dlg = cardDialogEl();
  const body = dlg.querySelector('.card-dialog-body');
  const sections = [...cardDialogContent().querySelectorAll('[data-cd-group]')];
  if (!sections.length) return;
  const top = body.getBoundingClientRect().top;
  const tops = sections.map(el => el.getBoundingClientRect().top - top + body.scrollTop);
  // At the very end the last group may be too short to reach the top.
  const atEnd = body.scrollTop > 0 && body.scrollTop + body.clientHeight >= body.scrollHeight - 2;
  const pinned = cardDialog.pinned && sections.find(el => el.dataset.cdGroup === cardDialog.pinned);
  const key = (pinned || sections[atEnd ? sections.length - 1 : cardDialogActiveGroup(tops, body.scrollTop)]).dataset.cdGroup;
  const changed = key !== cardDialog.active;
  cardDialog.active = key;
  if (changed) cardDialogFollowDock(key);
  let current = null;
  dlg.querySelectorAll('.card-dialog-index [data-cd-go-group]').forEach(b => {
    if (b.hasAttribute('data-cd-go-card')) return;
    if (b.dataset.cdGoGroup === key) { b.setAttribute('aria-current', 'location'); current = b; }
    else b.removeAttribute('aria-current');
  });
  if (changed && current) current.scrollIntoView({ block: 'nearest' });
  cardDialogSeeUpdated();
}
// A scroll by hand ends the wait for a target; the dialog's own scrolling does not come through these.
const cardDialogUserMoved = () => { cardDialog.target = null; cardDialog.pinned = null; };
// A press on the body can be the scrollbar.
for (const type of ['wheel', 'touchstart', 'pointerdown']) cardDialogEl().querySelector('.card-dialog-body').addEventListener(type, cardDialogUserMoved, { passive: true });
cardDialogEl().querySelector('.card-dialog-body').addEventListener('keydown', e => {
  if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(e.key)) cardDialogUserMoved();
});
cardDialogEl().querySelector('.card-dialog-body').addEventListener('scroll', e => {
  if (cardDialog.mode !== 'task') return;
  // Away from where a jump left it: the person has scrolled, however they did it.
  if (cardDialog.pinned && Math.abs(e.currentTarget.scrollTop - cardDialog.pinnedAt) > 8) cardDialog.pinned = null;
  if (cardDialog.spy) return;
  cardDialog.spy = true;
  requestAnimationFrame(() => { cardDialog.spy = false; cardDialogSpy(); });
});

/* The dialog of a task is open: what is shown here is not for the list behind it to move on from. */
function cardDialogTaskOpen() {
  return cardDialogEl().open && cardDialog.mode === 'task';
}

/* The target is brought into view once a card of it is drawn; while what it is in is still loading it waits. */
const CARD_DIALOG_TARGET_MS = 10000;
function cardDialogApplyTarget(task, all) {
  if (!cardDialog.target || !cardDialogEl().open) return;
  if (Date.now() - cardDialog.targetAt > CARD_DIALOG_TARGET_MS) { cardDialog.target = null; return; }
  const key = cardDialogResolveTarget(cardDialog.groups, cardDialog.target);
  if (key) { cardDialog.target = null; return void cardDialogGo(key, null, true); }
  const loading = !histories[historyKey(task, baseOf(task))]?.loaded || all.some(diffPending);
  if (!loading) cardDialog.target = null;
}

function openCardDialogTask(task, btn) {
  const dlg = cardDialogEl();
  const card = btn.closest('[data-expand]');
  const all = gatesOf(task);
  const first = cardDialogWaiting(all)[0];
  // A card of the panel names its section and gate; the head's button, the first gate that waits, or the top.
  const target = card ? { section: card.dataset.expand, gate: card.dataset.expandGate || null }
    : first ? { section: null, gate: gateRef(first) } : null;
  const dock = cardDialogDockGate(all, target?.gate);
  Object.assign(cardDialog, {
    mode: 'task', subject: selectedTaskId, pane: nav.pane, section: target?.section || null, gate: target?.gate || null,
    opener: btn, sig: '', held: false, quiet: false, gone: false, answering: false, gateGone: false, pressing: false,
    target, targetAt: Date.now(), pinned: null, dock: dock ? gateRef(dock) : null, dockSig: '', groups: [], gateIds: {}, active: null,
    answered: new Map(), answeredHere: new Set(), noMore: false, token: cardDialog.token + 1,
  });
  cardDialogShell('task', task.title, 'タスク全体');
  cardDialogSync(true, true);
  dlg.showModal();
  dlg.querySelector('.card-dialog-body').scrollTop = 0;
  cardDialogApplyTarget(task, all);
  if (!cardDialogContent().contains(document.activeElement)) dlg.querySelector('.card-dialog-body').focus({ preventScroll: true });
  cardDialogSpy();
}

/* What has focus in the dialog, as something that can be found again after the content is drawn anew. */
const CARD_DIALOG_FOCUSABLE = 'a[href], button, input, select, textarea, summary';
function cardDialogFocusOf(dlg) {
  const f = document.activeElement;
  if (!dlg.open || !f || !dlg.contains?.(f)) return null;
  if (f.matches('[data-cd-go-group]')) return { kind: 'go', group: f.dataset.cdGoGroup, card: f.dataset.cdGoCard || null };
  if (f.matches('[data-cd-card]')) return { kind: 'card', key: f.dataset.cdCard };
  if (f.matches('[data-cd-group]')) return { kind: 'group', key: f.dataset.cdGroup };
  // The column's own buttons, which sit outside the gate's controls.
  if (f.matches('[data-cd-dock]')) return { kind: 'dock', ref: f.dataset.cdDock };
  if (f.matches('.cd-dock-head [data-cd-goto]')) return { kind: 'dockgo', ref: f.dataset.cdGoto };
  const gate = f.closest('[data-gate]');
  if (!gate) {
    // Anything else focusable in a card: a checkbox, a link, a summary, a button.
    const card = f.closest('[data-cd-card]');
    const at = card ? [...card.querySelectorAll(CARD_DIALOG_FOCUSABLE)].indexOf(f) : -1;
    return at < 0 ? null : { kind: 'in', key: card.dataset.cdCard, at };
  }
  return { kind: 'gate', gate: gate.dataset.gate, box: f.matches('.gate-comment'), act: f.dataset.act || null, choice: f.dataset.choice || null };
}
function cardDialogFocusBack(dlg, d) {
  if (!d) return;
  const all = sel => [...dlg.querySelectorAll(sel)];
  const el = d.kind === 'go' ? all('.card-dialog-index [data-cd-go-group]').find(b => b.dataset.cdGoGroup === d.group && (b.dataset.cdGoCard || null) === d.card)
    : d.kind === 'in' ? [...(all('[data-cd-card]').find(c => c.dataset.cdCard === d.key)?.querySelectorAll(CARD_DIALOG_FOCUSABLE) || [])][d.at]
    : d.kind === 'card' ? all('[data-cd-card]').find(c => c.dataset.cdCard === d.key)
    : d.kind === 'group' ? all('[data-cd-group]').find(c => c.dataset.cdGroup === d.key)
    : d.kind === 'dock' ? all('[data-cd-dock]').find(b => b.dataset.cdDock === d.ref)
    : d.kind === 'dockgo' ? all('.cd-dock-head [data-cd-goto]').find(b => b.dataset.cdGoto === d.ref)
    : all('[data-gate]').filter(g => g.dataset.gate === d.gate).flatMap(g => [...g.querySelectorAll(d.box ? '.gate-comment' : 'button')])
      .find(b => d.box || ((b.dataset.act || null) === d.act && (b.dataset.choice || null) === d.choice));
  el?.focus({ preventScroll: true });
}

/* ---- What changed while the dialog is open (#630) ---- */

const CARD_DIALOG_UPDATE_MS = 6000;
const CARD_DIALOG_SEEN_MS = 1200;

const cardDialogUpdateEl = () => cardDialogEl().querySelector('.card-dialog-update');

/* The markup of each card with its relative times made alike: 「5分前」 becoming 「6分前」 is not a change. */
// A tick on a manual check is the person's own, and is in the markup once it is made.
const cardDialogSigOf = html => timeFree(html).replace(/<input\b[^>]*>/g, tag => tag.replace(/ checked(?=[ >])/g, ''));
const cardDialogCardSigs = groups => new Map(groups.flatMap(g => g.cards.map(c => [c.key, cardDialogSigOf(c.html)])));

/* What changed from the last drawing (`prevSigs`, from `cardDialogCardSigs`) to `groups`, in the order it is read:
   `{ cards: [key], groups: Set of keys, firstKey, newGates: [{ ref, key, label }] }`. A card is marked when it is new or
   its content differs. Not marked: anything in the first fill, what is loading or finished loading (a loading note, or what
   the history brings in once it has been read), and the cards of a gate answered here. What else the person's own answer
   changes is left to the caller, which does not ask (`cardDialog.rebase`). `answered` holds the new cards of answered gates. */
function cardDialogChanged(prevSigs, groups, { answeredHere } = {}) {
  const here = answeredHere || new Set();
  const out = { cards: [], groups: new Set(), firstKey: null, newGates: [], answered: [] };
  if (!prevSigs.size) return out;
  const loading = html => html != null && (html.includes(DIFF_LOADING) || html.includes(HISTORY_LOADING));
  // The answered gates, the review and the plan come from the history: they all appear once it has been read.
  const historyRead = loading(prevSigs.get('history:timeline')) && !loading(groups.flatMap(g => g.cards).find(c => c.key === 'history:timeline')?.html);
  const mark = (g, c) => { out.cards.push(c.key); out.groups.add(g.key); };
  for (const g of groups) {
    // A waiting gate that has no card in the last drawing has just opened.
    if (g.gate && !here.has(g.gate) && g.cards.every(c => !prevSigs.has(c.key))) {
      out.newGates.push({ ref: g.gate, key: g.key, label: g.label });
      g.cards.forEach(c => mark(g, c));
      continue;
    }
    if (historyRead && !g.gate && g.key !== 'details') continue;
    const loadedHere = g.cards.some(c => loading(prevSigs.get(c.key)));
    for (const c of g.cards) {
      if (c.gate && here.has(c.gate)) continue;
      const was = prevSigs.get(c.key);
      if (loading(was) || loading(c.html) || (was === undefined && loadedHere)) continue;
      if (was === undefined || was !== cardDialogSigOf(c.html)) {
        // A gate's own card, new, is the gate answered somewhere else.
        if (was === undefined && c.gate && c.key === `gate:${c.gate}`) out.answered.push(c.key);
        mark(g, c);
      }
    }
  }
  out.firstKey = out.cards[0] ?? null;
  return out;
}

/* What the notice says: the gates that joined the waiting ones, the gates answered elsewhere, then the cards that
   changed (as many as three by name), by label, once each. */
function cardDialogUpdateText(changes, groups, most = 3) {
  const fresh = new Set(changes.newGates.map(g => g.key));
  const own = new Set(groups.filter(g => fresh.has(g.key)).flatMap(g => g.cards.map(c => c.key)));
  const labelOf = new Map(groups.flatMap(g => g.cards.map(c => [c.key, c.label])));
  const done = new Set(changes.answered || []);
  const labels = [...new Set(changes.cards.filter(k => !own.has(k) && !done.has(k)).map(k => labelOf.get(k)).filter(Boolean))];
  const parts = [];
  if (changes.newGates.length) parts.push(`判断待ちに${changes.newGates.map(g => g.label).join('・')} が加わりました`);
  if (done.size) parts.push(`${[...done].map(k => labelOf.get(k)).filter(Boolean).join('・')} が回答済みになりました`);
  if (labels.length) parts.push(`${labels.slice(0, most).join('・')}${labels.length > most ? `ほか${labels.length - most}件` : ''}が更新されました`);
  return parts.join('。');
}

/* The marks (the 「更新」 badge on the index and the dock's switch, an outline on the card) are set on what is drawn,
   after every drawing and every clearing, and never put in the markup the redraw is compared on. Only the badges are
   added or taken away: the index is not drawn again for it. */
function cardDialogMarkUpdated() {
  const dlg = cardDialogEl();
  const up = cardDialog.updated;
  const inGroup = new Set(cardDialog.groups.filter(g => g.cards.some(c => up.has(c.key))).map(g => g.key));
  cardDialogContent().querySelectorAll('[data-cd-card]').forEach(el => {
    el.classList.toggle('cd-updated', up.has(el.dataset.cdCard));
    // The outline alone is a cue of colour; the word is the card's own.
    cardDialogBadge(el, up.has(el.dataset.cdCard), true);
  });
  dlg.querySelectorAll('.card-dialog-index [data-cd-go-group]').forEach(b =>
    cardDialogBadge(b, b.hasAttribute('data-cd-go-card') ? up.has(b.dataset.cdGoCard) : inGroup.has(b.dataset.cdGoGroup), false));
  cardDialogDockEl().querySelectorAll('[data-cd-dock]').forEach(b => cardDialogBadge(b, cardDialog.updatedDocks.has(b.dataset.cdDock), true));
}
/* `first`: ahead of the text, where a long name that is cut short does not hide it. */
function cardDialogBadge(btn, on, first) {
  const has = btn.querySelector('.cd-upd');
  if (on === !!has) return;
  if (has) return has.remove();
  const b = document.createElement('span');
  b.className = 'cd-badge cd-upd';
  b.textContent = '更新';
  if (first) btn.prepend(b); else btn.append(b);
}

/* A card is seen when it is wholly in the body, or enough of it is apart from the edges where it is only coming in. Once seen its
   mark goes after a moment, so that it is seen to go. */
function cardDialogSeeUpdated() {
  if (cardDialog.mode !== 'task' || !cardDialog.updated.size || document.hidden || !cardDialogEl().open) return;
  const body = cardDialogEl().querySelector('.card-dialog-body');
  const view = body.getBoundingClientRect();
  const edge = Math.min(60, body.clientHeight / 4);
  const token = cardDialog.token;
  for (const key of cardDialog.updated) {
    if (cardDialog.seenTimers.has(key)) continue;
    const el = cardDialogCardEl(key);
    if (!el) continue;
    const r = el.getBoundingClientRect();
    const shown = Math.min(r.bottom, view.bottom - edge) - Math.max(r.top, view.top + edge);
    // A card wholly in the body is seen wherever it is: a jump puts it at the very top, and the last (or first) card
    // cannot be scrolled out of the margins. Otherwise a card taller than half the body is seen when half the body of it is.
    const whole = r.top >= view.top && r.bottom <= view.bottom;
    if (!whole && shown < Math.min(r.height, body.clientHeight / 2)) continue;
    cardDialog.seenTimers.set(key, setTimeout(() => {
      if (token !== cardDialog.token) return;
      cardDialog.seenTimers.delete(key);
      cardDialog.updated.delete(key);
      cardDialogMarkUpdated();
    }, CARD_DIALOG_SEEN_MS));
  }
}
document.addEventListener('visibilitychange', () => { if (cardDialogEl().open) cardDialogSeeUpdated(); });

/* The notice under the head. It names what changed and goes after a few seconds, which wait while the pointer or
   focus is in it. */
function cardDialogShowUpdate(changes) {
  // What the notice still says stays in it: a second change adds its names, and 「見る」 goes to the first of all.
  const shown = cardDialog.updateText ? cardDialog.updateChanges : null;
  if (shown) changes = cardDialogMergeChanges(shown, changes, cardDialog.groups);
  if (cardDialogWriteUpdate(changes)) cardDialogUpdateTimer();
}
/* What the notice names, written into it; false (and the notice gone) when there is nothing left to name, so that
   `updateChanges` and the DOM never disagree. */
function cardDialogWriteUpdate(changes) {
  const text = cardDialogUpdateText(changes, cardDialog.groups);
  if (!text) {
    cardDialogHideUpdate();
    return false;
  }
  cardDialog.updateChanges = changes;
  const el = cardDialogUpdateEl();
  // The button stays where it is when the words change, so focus on it is not lost; the words are rewritten only when
  // they differ, as it is a live region.
  if (!el.querySelector('[data-cd-see]')) el.innerHTML = `<span></span><button type="button" data-cd-see="">見る</button>`;
  if (text !== cardDialog.updateText) {
    cardDialog.updateText = text;
    el.querySelector('span').textContent = text;
  }
  el.querySelector('[data-cd-see]').dataset.cdSee = changes.firstKey;
  return true;
}
/* A redraw took away what the notice names (a gate answered, a card gone) and brought nothing new: it says what is
   left, or goes. Its time is not started again, as nothing new was said. */
function cardDialogPruneUpdate() {
  if (!cardDialog.updateText || !cardDialog.updateChanges) return;
  cardDialogWriteUpdate(cardDialogMergeChanges(cardDialog.updateChanges, { cards: [], newGates: [], answered: [] }, cardDialog.groups));
}
/* `a` then `b`, each card once, in the order they are read in `groups`; a card that is gone is let go. */
function cardDialogMergeChanges(a, b, groups) {
  const order = groups.flatMap(g => g.cards.map(c => c.key));
  const cards = [...new Set([...a.cards, ...b.cards])].filter(k => order.includes(k)).sort((x, y) => order.indexOf(x) - order.indexOf(y));
  const gates = [...a.newGates, ...b.newGates.filter(n => !a.newGates.some(m => m.ref === n.ref))].filter(n => groups.some(g => g.key === n.key));
  const answered = [...new Set([...(a.answered || []), ...(b.answered || [])])].filter(k => cards.includes(k));
  return { cards, firstKey: cards[0] ?? null, newGates: gates, answered };
}
function cardDialogUpdateTimer() {
  clearTimeout(cardDialog.updateTimer);
  const token = cardDialog.token;
  cardDialog.updateTimer = setTimeout(() => {
    const el = cardDialogUpdateEl();
    if (token === cardDialog.token && !el.matches(':hover') && !el.contains(document.activeElement)) cardDialogHideUpdate();
  }, CARD_DIALOG_UPDATE_MS);
}
function cardDialogHideUpdate() {
  clearTimeout(cardDialog.updateTimer);
  cardDialog.updateTimer = 0;
  cardDialog.updateText = '';
  cardDialog.updateChanges = null;
  cardDialogUpdateEl().innerHTML = '';
}
// Pointer or focus in the notice holds it; leaving starts the time again.
cardDialogUpdateEl().addEventListener('pointerenter', () => clearTimeout(cardDialog.updateTimer));
cardDialogUpdateEl().addEventListener('focusin', () => clearTimeout(cardDialog.updateTimer));
for (const type of ['pointerleave', 'focusout']) {
  cardDialogUpdateEl().addEventListener(type, () => { if (cardDialog.updateText) cardDialogUpdateTimer(); });
}
/* 「見る」: the card that was named first, or the first still marked when it has gone. */
function cardDialogSeeNotice(key) {
  cardDialogHideUpdate();
  const at = cardDialogCardEl(key) ? key : [...cardDialog.updated].find(k => cardDialogCardEl(k));
  if (at) cardDialogGo(at, null, true);
}
/* Nothing marked, no notice, no timer: a new opening, and a closed one. */
function cardDialogResetUpdates() {
  cardDialogHideUpdate();
  cardDialog.seenTimers.forEach(t => clearTimeout(t));
  Object.assign(cardDialog, { cardSigs: new Map(), updated: new Set(), updatedDocks: new Set(), seenTimers: new Map(), waitingRefs: [], rebase: false });
}

/* The panel drew again: the dialog of a task follows. */
function cardDialogFollow() {
  if (cardDialog.mode !== 'task' || !cardDialogEl().open) return;
  cardDialogSync();
}

/* `cardDialogSync` for a task: draws the whole task from the data. `force` redraws although a comment is being
   typed (what was typed is carried over); `initial` is the first fill, before the dialog is open. */
function cardDialogTaskSync(force, initial) {
  const dlg = cardDialogEl();
  if (!dlg.open && !initial) return;
  // An answer on its way drops the gate before it is known to have gone: wait for the outcome.
  if (cardDialog.answering) return;
  const task = taskById(cardDialog.subject);
  // Checked first: dismissing the panel clears the selection too. What was shown stays, read-only.
  if (!task && state.now != null) {
    cardDialog.gone = true;
    return cardDialogStrip(CARD_DIALOG_TASK_GONE);
  }
  if (selectedTaskId !== cardDialog.subject || tp('task-panel').hidden) {
    cardDialog.quiet = true;
    if (dlg.open) dlg.close();
    return;
  }
  // The board has not answered yet: what it lacks is not known to be missing.
  if (!task) return;
  const content = cardDialogContent();
  const col = cardDialogDockEl();
  const title = dlg.querySelector('#card-dialog-title');
  if (title.textContent !== task.title) title.textContent = task.title;
  // The history's record taking the place of an answered gate's copy changes what is drawn, with nothing new in it.
  const copies = cardDialog.answered.size;
  const all = cardDialogWithAnswered(gatesOf(task), cardDialog.answered);
  // Kept on the dialog, not here: a sync held while typing must not lose it.
  if (cardDialog.answered.size !== copies) cardDialog.rebase = true;
  const waiting = cardDialogWaiting(all);
  const isWaitingRef = ref => waiting.some(g => gateRef(g) === ref);

  // What was typed, for each gate that has a box here (the panel's, at the first fill). Every waiting gate has its own
  // dock, shown or not.
  const typed = new Map();
  if (dlg.open) col.querySelectorAll('[data-gate]').forEach(el => {
    const box = el.querySelector('.gate-comment');
    if (box) typed.set(el.dataset.gate, box.value);
  });
  else {
    // A draft of one of this task's gates that no longer waits is let go; another task's stays.
    for (const g of all) if (!isWaitingRef(gateRef(g))) cardDialogDrafts.delete(gateRef(g));
    // The panel's box says what it says, even when empty; the kept draft is for a gate the panel has no box for.
    waiting.forEach(g => {
      const box = cardDialogPanelComment(gateRef(g));
      typed.set(gateRef(g), box ? box.value : cardDialogDrafts.get(gateRef(g)) || '');
    });
  }

  // A gate answered elsewhere, with something typed for it: its dock stays as it was, read-only. One answered from
  // here is not that, and its comment has been sent.
  if (dlg.open) col.querySelectorAll('[data-cd-dock-for]').forEach(el => {
    const ref = el.dataset.cdDockFor;
    if (isWaitingRef(ref) || cardDialog.answeredHere.has(ref) || !typed.get(ref)) return;
    cardDialogStripEl(el.querySelector('[data-gate]'));
    el.removeAttribute('data-cd-dock-for');
    el.classList.add('cd-dock-gone');
    el.hidden = false;
    cardDialog.dockSig = '';
  });
  const kept = dlg.open ? [...col.querySelectorAll('.cd-dock-gone')] : [];
  const shown = cardDialog.dock;
  const dock = cardDialogDockGate(all, cardDialog.dock);
  cardDialog.dock = dock ? gateRef(dock) : null;
  // The dock fell back to the first waiting gate: it may not be the one of the group in view.
  const fellBack = !!shown && !!dock && cardDialog.dock !== shown;
  // The last waiting gate was answered somewhere else: the same words as after answering it here.
  if (cardDialog.waitingRefs.length && !waiting.length) cardDialog.noMore = true;
  if (waiting.length) cardDialog.noMore = false;
  cardDialog.waitingRefs = waiting.map(gateRef);
  cardDialogNotice(cardDialogNoticeText(kept, waiting));

  if (!force && dlg.open && document.activeElement?.matches('#card-dialog .gate-comment')) {
    cardDialog.held = true;
    return;
  }
  cardDialog.held = false;
  cardDialog.gone = false;
  const rebase = cardDialog.rebase;
  cardDialog.rebase = false;

  const groups = cardDialogTaskGroups(task, all);
  const html = cardDialogTaskHtml(groups);
  const sig = `${html.index}\u0000${html.content}`;
  const dockHtml = cardDialogDockHtml(waiting);
  cardDialog.groups = groups;
  cardDialog.gateIds = Object.fromEntries(all.map(g => [g.id, gateRef(g)]));
  const contentSame = sig === cardDialog.sig && !force;
  const dockSame = dockHtml === cardDialog.dockSig && !force;
  if (contentSame && dockSame) {
    cardDialogShowDock();
    if (fellBack) cardDialogFollowDock(cardDialog.active);
    return cardDialogApplyTarget(task, all);
  }
  cardDialog.sig = sig;
  // Against what was last drawn, so a redraw held while typing is marked once, with what came after it. The first
  // fill, and what the person's own answer brought, are not changes.
  let changes = null;
  if (!contentSame) {
    if (!initial && !rebase) changes = cardDialogChanged(cardDialog.cardSigs, groups, { answeredHere: cardDialog.answeredHere });
    cardDialog.cardSigs = cardDialogCardSigs(groups);
    const keys = new Set(cardDialog.cardSigs.keys());
    cardDialog.updated = new Set([...cardDialog.updated].filter(k => keys.has(k)));
    if (changes) {
      changes.cards.forEach(k => {
        cardDialog.updated.add(k);
        // A card seen a moment ago that has changed again is not seen yet: its old timer must not take the new mark.
        clearTimeout(cardDialog.seenTimers.get(k));
        cardDialog.seenTimers.delete(k);
      });
      changes.newGates.forEach(g => cardDialog.updatedDocks.add(g.ref));
    }
  }

  const body = dlg.querySelector('.card-dialog-body');
  // What the person has open, and where they are reading, survive the redraw.
  const opened = new Map();
  if (dlg.open) content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => opened.set(`${c.dataset.cdCard}#${i}`, d.open)));
  const scroll = body.scrollTop;
  const top = body.getBoundingClientRect().top;
  const first = dlg.open ? [...content.querySelectorAll('[data-cd-card]')].find(c => c.getBoundingClientRect().bottom > top + 1) : null;
  const anchor = first && { key: first.dataset.cdCard, offset: first.getBoundingClientRect().top - top };

  const focus = cardDialogFocusOf(dlg);
  if (!contentSame) {
    dlg.querySelector('.card-dialog-index').innerHTML = html.index;
    content.replaceChildren(cardDialogFill(html.content, groups, id => cardDialog.gateIds[id] || null));
  }
  if (!dockSame) {
    cardDialog.dockSig = dockHtml;
    col.replaceChildren(...kept, cardDialogFill(dockHtml, groups, id => cardDialog.gateIds[id] || null));
    col.querySelectorAll('[data-gate]').forEach(el => {
      const box = el.querySelector('.gate-comment');
      if (box && typed.get(el.dataset.gate)) box.value = typed.get(el.dataset.gate);
    });
  }
  // The column's width is part of the content's, so it is settled before the scroll is put back.
  cardDialogShowDock();
  if (fellBack) cardDialogFollowDock(cardDialog.active);
  content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => {
    const was = opened.get(`${c.dataset.cdCard}#${i}`);
    if (was !== undefined) d.open = was;
  }));
  body.scrollTop = scroll;
  const same = anchor && cardDialogCardEl(anchor.key);
  if (same) body.scrollTop += same.getBoundingClientRect().top - body.getBoundingClientRect().top - anchor.offset;
  cardDialogFocusBack(dlg, focus);
  if (cardDialog.pinned) cardDialog.pinnedAt = body.scrollTop;
  cardDialogMarkUpdated();
  if (dlg.open) {
    if (changes?.cards.length) cardDialogShowUpdate(changes);
    else cardDialogPruneUpdate();
    cardDialogApplyTarget(task, all);
    cardDialogSpy();
  }
}
