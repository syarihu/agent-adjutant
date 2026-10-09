/* 「拡大」: a large dialog to read in. For a task it holds the whole task in one scroll, drawn from the data
   with the panel's own card renderers (task-view.js, decide.js), under a left index of what is in it, and one
   answer band for one waiting gate; answering closes the dialog. For what has no task (a hub, a session, a
   gate of its own) it holds one card alone, copied from the panel's DOM, which it follows while it is open and
   which brings its gate's answer controls along. */

const cardDialog = {
  subject: null, pane: null, section: null, gate: null, opener: null,
  sig: '', held: false, quiet: false, gone: false, pending: false, observer: null,
  expectControls: false, token: 0, pressing: false, answering: false, gateGone: false,
  // The whole task: `mode` is 'task' then, `target` the card to bring into view once it is drawn, `dock` the ref of
  // the gate the answer band is for, `groups` what was drawn last, `active` the group the index marks.
  mode: 'card', target: null, targetAt: 0, pinned: null, pinnedAt: 0, dock: null, groups: [], gateIds: {}, active: null, spy: false,
};
const CARD_DIALOG_GONE = 'パネルからこのカードがなくなりました（最後に表示した内容です）';
const CARD_DIALOG_GATE_GONE = 'この gate への答えはパネルから消えました（最後に表示した内容です）';
const CARD_DIALOG_TASK_GONE = 'パネルからこのタスクがなくなりました（最後に表示した内容です）';
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
  cardDialogContent().querySelectorAll('[data-gate]').forEach(cardDialogStripEl);
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
    answering: false, gateGone: false, pressing: false, target: null, dock: null, groups: [],
    token: cardDialog.token + 1,
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
  // Nothing of an earlier opening may be read back: what is kept across redraws is read from this DOM.
  dlg.querySelector('.card-dialog-body').style.removeProperty('--cd-dock-h');
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
  // The index and the timeline move inside the dialog; nothing is sent.
  if ((b = hit('[data-cd-go-group]'))) return cardDialogGo(b.dataset.cdGoCard || null, b.dataset.cdGoGroup, false);
  if ((b = hit('[data-cd-goto]'))) return cardDialogGoGate(b.dataset.cdGoto);
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
  // What was typed goes back to the box of the same gate in the panel, and to no other.
  if (selectedTaskId === cardDialog.subject) {
    cardDialogEl().querySelectorAll('[data-gate]').forEach(el => {
      const typed = el.querySelector('.gate-comment')?.value;
      const panelBox = cardDialogPanelComment(el.dataset.gate);
      if (panelBox && typed != null) panelBox.value = typed;
    });
  }
  if (!cardDialog.quiet) cardDialogFocusTarget()?.focus();
  cardDialog.subject = null; cardDialog.opener = null; cardDialog.sig = ''; cardDialog.held = false;
  cardDialog.quiet = false; cardDialog.gone = false; cardDialog.expectControls = false; cardDialog.pressing = false; cardDialog.answering = false; cardDialog.gateGone = false;
  cardDialog.target = null; cardDialog.pinned = null; cardDialog.dock = null; cardDialog.groups = []; cardDialog.gateIds = {}; cardDialog.active = null;
  // What was drawn goes with the opening, and so does its notice.
  cardDialogContent().replaceChildren();
  cardDialogEl().querySelector('.card-dialog-index').replaceChildren();
  cardDialogEl().querySelector('.card-dialog-gone').textContent = '';
  cardDialogEl().querySelector('.card-dialog-body').style.removeProperty('--cd-dock-h');
});

/* ---- The whole task ---- */

/* The task the panel shows, if it shows one; what has none keeps the single-card path. */
const cardDialogTask = () => selectedTaskId && !isHubRef(selectedTaskId) ? taskById(selectedTaskId) : null;

/* The waiting gates of the task, oldest first, with their open time as `gatesOf` orders them. */
const cardDialogWaiting = all => all.filter(g => g.wait !== false && isWaiting(g));

/* What the dialog holds, as groups of cards, in the order they are read: what waits, the review, the diff, the
   plan, what was answered, 経過, 詳細. A group or card with nothing to say is left out, and so are the panel's 記録 list
   and 工程. `dock` is the gate the answer band is for: the only one whose choices can be picked here.
   `{ key, label, badge, cards: [{ key, label, badge, wide, html, expand: [section, gateRef] | null }] }`. */
function cardDialogTaskGroups(task, all, dock = null) {
  const dockRef = dock ? gateRef(dock) : null;
  const card = (key, label, html, o = {}) => html ? { key, label, badge: o.badge || '', wide: !!o.wide, html, expand: o.expand || null } : null;
  const at = (section, g) => [section, g ? gateRef(g) : null];
  const decidedHtml = g => g.decided ? `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<h3>決定事項</h3><div class="body">${md(g.decided)}</div></div>` : '';
  const runHtml = g => g.run ? `<div class="panel"${expandAttrs('run', g)}>${expandBtnHtml('動かし方')}<h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>` : '';
  const group = (key, label, cards, badge = '') => {
    const kept = cards.filter(Boolean);
    return kept.length ? { key, label, badge, cards: kept } : null;
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
      card(k('choices'), '選択肢', choicesHtml(g, ref === dockRef), { wide: true, expand: at('choices', g) }),
    ];
    if (g.kind === 'verify') {
      cards.push(card(k('commands'), 'Verify 実行結果', commandsCardHtml(g), { expand: at('commands', g) }),
        card(k('manual'), '人が見る確認項目', manualCardHtml(g), { expand: at('manual', g) }),
        card(k('run'), '動かし方', runHtml(g), { expand: at('run', g) }));
    }
    // The review and the diff of another waiting gate of the kind: group 2 and 3 hold only one.
    if (g.kind === 'diff' && g !== gR) cards.push(...reviewCards(g, `gate:${ref}`), ...diffCards(g, `gate:${ref}`));
    if (g.kind !== 'plan') cards.push(card(k('decided'), '決定事項', decidedHtml(g), { expand: at('decided', g) }));
    return group(`waiting:${ref}`, `【${kindOf(g.kind)[0]}】${g.title}`, cards, '判断待ち');
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

  // 5. the gates answered: one card for each
  const answered = all.filter(g => !isWaiting(g) && g.wait !== false).map(g => {
    const html = gateHeadHtml(g, [g]) + framesHtml(g) + reportCardHtml(g) + choicesHtml(g, false) +
      (g.kind === 'verify' ? checkPanels(g) + runHtml(g) : g.kind === 'diff' && g !== gR ? reviewPanels(g) : '') +
      // The plan shown in the plan group has its 決定事項 there.
      (g === plan ? '' : decidedHtml(g));
    return card(`gate:${gateRef(g)}`, `【${kindOf(g.kind)[0]}】${g.title}`, html, { badge: DECISION[g.decision] || '回答済み', expand: [null, gateRef(g)] });
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

/* The gate the answer band is for: the opened card's gate if it waits, else the first that waits. */
function cardDialogDockGate(all, ref) {
  const waiting = cardDialogWaiting(all);
  return (ref && waiting.find(g => gateRef(g) === ref)) || waiting[0] || null;
}

/* The index and the content, as markup. Every label and key is escaped; a key is only ever an attribute value. */
function cardDialogTaskHtml(groups, dockGate) {
  const entry = (g, c) => `<li><button type="button" data-cd-go-group="${esc(g.key)}"${c ? ` data-cd-go-card="${esc(c.key)}"` : ''}>` +
    `<span>${esc((c || g).label)}</span>${(c || g).badge ? `<span class="cd-badge">${esc((c || g).badge)}</span>` : ''}</button>`;
  const index = `<ol>` + groups.map(g => entry(g) + `<ol>${g.cards.map(c => entry(g, c) + '</li>').join('')}</ol></li>`).join('') + `</ol>`;
  const content = groups.map(g => `<section class="cd-group" data-cd-group="${esc(g.key)}" aria-label="${esc(g.label)}" tabindex="-1">` +
    `<h3 class="cd-group-title"><span>${esc(g.label)}</span>${g.badge ? `<span class="cd-badge">${esc(g.badge)}</span>` : ''}</h3>` +
    `<div class="cd-grid">${g.cards.map(c => `<div class="cd-card${c.wide ? ' cd-wide' : ''}" data-cd-card="${esc(c.key)}" tabindex="-1">${c.html}</div>`).join('')}</div></section>`).join('') +
    (dockGate ? `<div class="cd-dock" role="group" aria-label="${esc(`【${kindOf(dockGate.kind)[0]}】${dockGate.title} に答える`)}"><p class="cd-dock-label">【${esc(kindOf(dockGate.kind)[0])}】${esc(dockGate.title)} に答える</p>${decideHtml(dockGate)}</div>` : '');
  return { index, content };
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
/* The room the answer band takes at the bottom, so a focused element scrolls to above it. */
function cardDialogDockSpace(body) {
  const docks = [...cardDialogContent().querySelectorAll('.cd-dock')].filter(el => !el.querySelector('[data-gate-gone]'));
  const h = docks.length ? Math.ceil(docks[docks.length - 1].getBoundingClientRect?.().height || 0) : 0;
  body.style.setProperty('--cd-dock-h', `${h}px`);
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
  cardDialogDockSpace(body);
  const changed = key !== cardDialog.active;
  cardDialog.active = key;
  let current = null;
  dlg.querySelectorAll('.card-dialog-index [data-cd-go-group]').forEach(b => {
    if (b.hasAttribute('data-cd-go-card')) return;
    if (b.dataset.cdGoGroup === key) { b.setAttribute('aria-current', 'location'); current = b; }
    else b.removeAttribute('aria-current');
  });
  if (changed && current) current.scrollIntoView({ block: 'nearest' });
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
    target, targetAt: Date.now(), pinned: null, dock: dock ? gateRef(dock) : null, groups: [], gateIds: {}, active: null,
    token: cardDialog.token + 1,
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
    : all('[data-gate]').filter(g => g.dataset.gate === d.gate).flatMap(g => [...g.querySelectorAll(d.box ? '.gate-comment' : 'button')])
      .find(b => d.box || ((b.dataset.act || null) === d.act && (b.dataset.choice || null) === d.choice));
  el?.focus({ preventScroll: true });
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
  const title = dlg.querySelector('#card-dialog-title');
  if (title.textContent !== task.title) title.textContent = task.title;
  const all = gatesOf(task);
  const waiting = cardDialogWaiting(all);

  // What was typed, for each gate that has a box here (the panel's, at the first fill).
  const typed = new Map();
  if (dlg.open) content.querySelectorAll('[data-gate]').forEach(el => {
    const box = el.querySelector('.gate-comment');
    if (box) typed.set(el.dataset.gate, box.value);
  });
  else if (cardDialog.dock) typed.set(cardDialog.dock, cardDialogPanelComment(cardDialog.dock)?.value || '');

  // The gate of the answer band is no longer waiting: with something typed, its band stays as it was, read-only.
  if (cardDialog.dock && !waiting.some(g => gateRef(g) === cardDialog.dock) && dlg.open && typed.get(cardDialog.dock)) {
    content.querySelectorAll('[data-gate]').forEach(el => {
      if (el.dataset.gate !== cardDialog.dock) return;
      cardDialogStripEl(el);
    });
    cardDialog.sig = '';
  }
  const dock = cardDialogDockGate(all, cardDialog.dock);
  cardDialog.dock = dock ? gateRef(dock) : null;
  const kept = !dlg.open ? [] : [...content.querySelectorAll('.cd-dock')].filter(el => el.querySelector('[data-gate-gone]'));
  cardDialogNotice(kept.length ? CARD_DIALOG_GATE_GONE : '');

  if (!force && dlg.open && document.activeElement?.matches('#card-dialog .gate-comment')) {
    cardDialog.held = true;
    return;
  }
  cardDialog.held = false;
  cardDialog.gone = false;

  const groups = cardDialogTaskGroups(task, all, dock);
  const html = cardDialogTaskHtml(groups, dock);
  const sig = `${html.index}\u0000${html.content}`;
  cardDialog.groups = groups;
  cardDialog.gateIds = Object.fromEntries(all.map(g => [g.id, gateRef(g)]));
  if (sig === cardDialog.sig && !force) return cardDialogApplyTarget(task, all);
  cardDialog.sig = sig;

  const body = dlg.querySelector('.card-dialog-body');
  // What the person has open, and where they are reading, survive the redraw.
  const opened = new Map();
  if (dlg.open) content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => opened.set(`${c.dataset.cdCard}#${i}`, d.open)));
  const scroll = body.scrollTop;
  const top = body.getBoundingClientRect().top;
  const first = dlg.open ? [...content.querySelectorAll('[data-cd-card]')].find(c => c.getBoundingClientRect().bottom > top + 1) : null;
  const anchor = first && { key: first.dataset.cdCard, offset: first.getBoundingClientRect().top - top };

  const focus = cardDialogFocusOf(dlg);
  dlg.querySelector('.card-dialog-index').innerHTML = html.index;
  const nodes = cardDialogFill(html.content, groups, id => cardDialog.gateIds[id] || null);
  const fresh = nodes.querySelector('.cd-dock');
  kept.forEach(k => fresh ? fresh.before(k) : nodes.append(k));
  content.replaceChildren(nodes);
  content.querySelectorAll('[data-gate]').forEach(el => {
    const box = el.querySelector('.gate-comment');
    if (box && typed.get(el.dataset.gate)) box.value = typed.get(el.dataset.gate);
  });
  content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => {
    const was = opened.get(`${c.dataset.cdCard}#${i}`);
    if (was !== undefined) d.open = was;
  }));
  body.scrollTop = scroll;
  const same = anchor && cardDialogCardEl(anchor.key);
  if (same) body.scrollTop += same.getBoundingClientRect().top - body.getBoundingClientRect().top - anchor.offset;
  cardDialogFocusBack(dlg, focus);
  if (cardDialog.pinned) cardDialog.pinnedAt = body.scrollTop;
  if (dlg.open) { cardDialogApplyTarget(task, all); cardDialogSpy(); }
}
