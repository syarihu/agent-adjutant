/* ── The task panel ────────────────────────────────────────────────────────────────────────
   One task at a time, beside the sidebar: `selectedTaskId` is the task it shows and `nav.pane`
   the tab. Where it sits is only a class on body (panel-right, panel-pop), so the terminal in
   #tp-term-host is never moved, rebuilt or redrawn: a terminal that is moved reconnects. The
   redraws below touch the parts around it, and never the host or what holds it. */
const tp = id => document.getElementById(id);
const taskById = id => (state.tasks || []).find(t => t.id === id);

/* The gate each tab of the open task shows when a person picked one in 経過 (keyed by the tab
   renderers' names, task-view.js). It belongs to one task: another task starts with none. */
let panelPick = { id: null, pick: {} };
function panelPickOf(id) {
  if (panelPick.id !== id) panelPick = { id, pick: {} };
  return panelPick.pick;
}

/* The panel's tabs are named by the tab renderers' (task-view.js) names, 概要 being タスクサマリ, whose
   address stays `detail`. */
const paneOfTab = tab => tab === 'overview' ? 'detail' : tab;
const tabOfPane = pane => pane === 'detail' ? 'overview' : pane;
/* The tab a gate is judged in. Read inside functions only: the table is in task-view.js, which
   loads after this file. */
const paneOfGate = g => paneOfTab(TAB_OF_KIND[g.kind] || 'history');
/* The board's own record behind a gate of the task, found by the task's board as the card's chips
   name it: a copy from the history may lack its board, and ids are only unique within one. */
const liveRecordOf = (task, g) => recordByRef(gateRef(task._slug && !g._slug ? { ...g, _slug: task._slug } : g));

/* ── A gate in the panel ─────────────────────────────────────────────────────────────────────
   A gate opens in the panel as `gate:<board>/<id>` (`gate:<id>` on a board served alone). One whose
   task is on this board is that task's panel, on the tab the gate is judged in, with the gate
   picked (`landGateRef`); one with no task card is judged in a panel of its own. */

/* The open gate a `gate:` subject names, from the state of the board that is shown. */
function gateOfPanelRef(ref) {
  if (!isGateRef(ref)) return null;
  const rest = ref.slice(WORK_GATE_REF.length);
  const at = rest.indexOf('/');
  const slug = at > 0 ? rest.slice(0, at) : null;
  const id = at > 0 ? rest.slice(at + 1) : rest;
  // The state is one board's: a gate of another board is not in it.
  if (slug && multiBoard && nav.board && nav.board !== 'all' && slug !== nav.board) return null;
  return (state.gates || []).find(g => g.id === id && (!g._slug || !slug || g._slug === slug)) || null;
}

/* A `gate:` subject whose gate has its task on this board becomes that task, with the tab and the
   gate picked as a link to the gate would have them: returns the task's id and the pane, or null
   for any other subject. The task is selected here, so that opening it does not forget the pick. */
function landGateRef(ref) {
  const g = gateOfPanelRef(ref);
  const task = g?.task && taskOfGate(g);
  if (!task) return null;
  selectedTaskId = task.id;
  panelPickOf(task.id)[tabOfPane(paneOfGate(g))] = g.id;
  panelScrolledFor = null;
  return { id: task.id, pane: paneOfGate(g) };
}

function markSelectedCards() {
  for (const card of document.querySelectorAll('#boards .card')) {
    card.classList.toggle('selected', card.dataset.id === selectedTaskId);
  }
}

/* The panel on `id`, as the address says: no history entry is made, the address is what asked. */
function showTaskPanel(id) {
  pendingTask = null;
  if (selectedTaskId !== id) { sessView.git = null; panelPick = { id: null, pick: {} }; dialogFor = dialogReturn = null; }
  selectedTaskId = id;
  markSelectedCards();
  renderTaskPanel();
}

/* A click on a card or one of its buttons: another card is a step in the history, the other tab
   of the card that is open replaces the one it is on. */
function openTaskPanel(id, pane = 'detail') {
  pendingTask = null;
  if (selectedTaskId !== id) { sessView.git = null; panelPick = { id: null, pick: {} }; dialogFor = dialogReturn = null; }
  selectedTaskId = id;
  markSelectedCards();
  go({ task: id, pane }, { replace: id === nav.task });
}

/* A link from outside the panel (a gate or record on a card, a notification):
   the panel on `tab` of the task, with `gateId` shown in it. There is more to read than the
   sidebar fits, so it opens as the dialog; the saved placement is not changed, and the next
   card or the close puts it back (panelPop). */
function openTask(id, tab = 'overview', gateId = null) {
  pendingTask = null;
  if (selectedTaskId !== id) { sessView.git = null; panelPick = { id: null, pick: {} }; }
  if (gateId) {
    panelPickOf(id)[tab] = gateId;
    // A gate picked is read from the top.
    panelScrolledFor = null;
  }
  // The dialog covers what was clicked: focus goes into it, and back when it closes (hideTaskPanelState).
  // A link followed inside a dialog already open keeps where that one came from.
  const from = document.activeElement;
  const inPanel = tp('task-panel').contains(from);
  if (!(dialogFor !== null && inPanel)) dialogReturn = from && from !== document.body && !inPanel ? from : null;
  selectedTaskId = id;
  // The work view keeps the panel in its right column: nothing there is a dialog.
  dialogFor = view === 'work' ? null : id;
  markSelectedCards();
  go({ task: id, pane: paneOfTab(tab) }, { replace: id === nav.task });
  requestAnimationFrame(() => {
    const at = document.activeElement;
    if (selectedTaskId === id && (at === from || at === document.body))
      tp('tp-tabs').querySelector('[aria-selected="true"]')?.focus();
  });
}

/* Whether the panel is the dialog now: the placement saved, or a link that wants room. */
let dialogFor = null;
let dialogReturn = null;
const panelPop = () => view !== 'work' && (prefs.panelDialog || (dialogFor !== null && dialogFor === selectedTaskId));

/* The panel's state, without the address: for a move the address already made. */
function hideTaskPanelState() {
  const card = selectedTaskId;
  // Only a close the person made: focus in the dialog, or dropped to the page by a click on the scrim.
  const at = document.activeElement;
  const back = dialogFor !== null && (!at || at === document.body || tp('task-panel').contains(at));
  const opener = dialogReturn;
  dialogReturn = null;
  pendingTask = null;
  selectedTaskId = null;
  panelScrolledFor = null;
  // Opened again, a session's git state is read again.
  sessView.git = null;
  panelPick = { id: null, pick: {} };
  dialogFor = null;
  // Drawn again when opened, so what was typed in the body does not come back.
  panelSig.links = panelSig.gate = panelSig.rest = null;
  panelShown = null;
  panelHeld = false;
  disposePanelTerminal();
  markSelectedCards();
  renderTaskPanel();
  // What opened the dialog may be gone (the board it was opened from left behind): then the
  // task's card, if it is on screen and the only one with that id (ids repeat across boards).
  if (back) {
    const shown = el => el?.isConnected && el.getClientRects().length > 0;
    const cards = [...document.querySelectorAll('#boards .card')].filter(c => c.dataset.id === card);
    const to = shown(opener) ? opener : cards.length === 1 ? cards[0].querySelector('button') : null;
    if (shown(to)) to.focus();
  }
}

/* A panel whose subject is gone: the address follows it. */
function dismissTaskPanel() {
  hideTaskPanelState();
  if (nav.task) setNav({ task: null, pane: 'detail' });
}

function closeTaskPanel() {
  hideTaskPanelState();
  // The work view's list belongs to no board: closing the selection goes back to it, as the sidebar's entry does.
  if (view === 'work') return go({ board: 'all', task: null, pane: 'detail' });
  if (nav.task) go({ task: null, pane: 'detail' });
}

/* Where panels open is a remembered mode: 'pop' is the dialog, 'left' and 'right' the sidebar on that side.
   Closing never changes it; only choosing one of these does. */
function placePanel(where) {
  dialogFor = dialogReturn = null;
  if (where === 'pop') prefs.panelDialog = true;
  else {
    prefs.panelDialog = false;
    prefs.panelDock = where;
  }
  savePrefs();
  renderTaskPanel();
}

/* A session is there to open a terminal on when its worktree has one at all. */
const hasSession = s => !!s && sessionState(s) !== 'none';
/* The tab shown: ターミナル only where there is a session, whatever the address says. */
const paneOf = task => nav.pane === 'term' ? (view !== 'work' && hasSession(sessionOfTask(task)) ? 'term' : 'detail')
  : PANES.includes(nav.pane) ? nav.pane : 'detail';
/* The same for a session with no task, which is its own subject. */
const sessPaneOf = s => nav.pane === 'term' && view !== 'work' && hasSession(s) ? 'term' : 'detail';

/* The sidebar is the icon rail when the window is narrow, and while the panel sits on its left. */
const narrowRail = matchMedia('(max-width: 1024px)');
function applyRailMode() {
  const cls = document.body.classList;
  const byPanel = cls.contains('panel-open') && !cls.contains('panel-right') && !cls.contains('panel-pop');
  cls.toggle('rail-icons', narrowRail.matches || byPanel);
}
narrowRail.addEventListener('change', applyRailMode);
// Before the first poll has drawn anything, a narrow window already has its icon rail.
applyRailMode();

/* What each part was last drawn from: a part is drawn again only when it changed. */
let panelScrolledFor = null;
/* The task, tab and gate the task's tabs were last drawn for: a comment being typed and the open
   `details` are carried across a redraw of the same one, and a scroll across the same tab. */
let panelShown = null;
let panelTabFor = null;
/* A redraw that came while a comment was being typed in the panel, held until the box is left.
   The minute labels change the markup at least once a minute, and a box built again loses what
   is typed in it and cuts an IME composition short. */
let panelHeld = false;
const panelCommentFocused = () => !!document.activeElement?.matches('#task-panel .gate-comment');
const panelSig = { head: '', tabs: '', gate: '', rest: '', bar: '', ph: '' };
function setPanelPart(part, el, html) {
  if (panelSig[part] === html) return;
  panelSig[part] = html;
  el.innerHTML = html;
}

function renderTaskPanel() {
  const panel = tp('task-panel');
  if (!panel) return;
  // A session that has been linked to a task since is shown as that task, and the address follows;
  // its terminal is kept (syncTermSlot).
  if (isSessRef(selectedTaskId)) {
    const ref = panelRefOf(selectedTaskId);
    if (ref !== selectedTaskId) {
      if (nav.task === selectedTaskId) setNav({ task: ref });
      selectedTaskId = ref;
      markSelectedCards();
    }
  }
  if (isGateRef(selectedTaskId)) {
    const from = selectedTaskId;
    const landed = landGateRef(from);
    if (landed) {
      if (nav.task === from) setNav({ task: landed.id, pane: landed.pane });
      markSelectedCards();
    }
  }
  const hub = hubOfRef(selectedTaskId);
  const task = selectedTaskId && !isHubRef(selectedTaskId) ? taskById(selectedTaskId) : null;
  const sess = !hub && !task ? sessOfRef(selectedTaskId) : null;
  const parent = view === 'work' ? parentOfRef(selectedTaskId) : null;
  // A gate with no task card is judged in a panel of its own.
  const gateOf = !hub && !task && !sess ? gateOfPanelRef(selectedTaskId) : null;
  // The work view marks the row, and draws the terminal, of what the address names.
  // The middle terminal is the work view's: one the panel mounted earlier is not kept beside it.
  if (view === 'work') {
    if (panelTerm.term) disposePanelTerminal();
    syncWorkSelection();
  }
  // A card the board no longer lists, a hub that left its list, or a session that is gone takes
  // the panel with it. In the work view the panel stays and says so: the row was opened from a
  // list that may know more than the board it is on.
  // A parent is not gone while the work document that lists it has not been read.
  const gone = !!selectedTaskId && !task && !hub && !sess && !parent && !gateOf && !(isParentRef(selectedTaskId) && !work.doc);
  if (gone && view !== 'work') return dismissTaskPanel();
  // Until the board has answered once, what it lacks is not known to be missing.
  if (gone && state.now == null) return;
  const shown = (!!(task || hub || sess || parent || gateOf) || gone) && (view === 'board' || view === 'work');
  const cls = document.body.classList;
  panel.hidden = !shown;
  tp('tp-scrim').hidden = !(shown && panelPop());
  cls.toggle('panel-open', shown);
  // The work view keeps the panel in its right column, whichever side it is docked on elsewhere.
  cls.toggle('panel-right', prefs.panelDock === 'right' || view === 'work');
  cls.toggle('panel-pop', shown && panelPop());
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
  applyRailMode();
  markHubButtons();
  // Closed, not just out of view: what was typed for the task goes with it.
  if (!task && !hub && !sess && !parent && !gateOf && !gone) return renderHandForm(null);
  if (!shown) return;
  if (gateOf) {
    renderWorkAway(null);
    return renderGatePanel(gateOf);
  }
  if (parent || gone) {
    renderWorkAway(null);
    return renderWorkPanelOnly(parent);
  }
  tp('tp-tabs').hidden = false;

  const colId = task ? columnOf(task) : null;
  const gate = task ? openGate(task) : null;
  const s = hub ? hubSessionOf(hub) : sess || sessionOfTask(task);
  const pane = hub ? hubPaneOf(hub, s) : sess ? sessPaneOf(s) : paneOf(task);
  // The gate the open tab shows, read before the tabs are drawn so the record on screen does not
  // keep its 新着.
  const all = task ? gatesOf(task) : [];
  if (wantedGate && (nav.board !== wantedGate.board || nav.task !== wantedGate.task)) wantedGate = null;
  if (task && wantedGate?.task === task.id) {
    const wanted = all.find(x => x.id === wantedGate.gate);
    if (wanted) {
      panelPickOf(task.id)[tabOfPane(paneOfGate(wanted))] = wanted.id;
      panelScrolledFor = null;
      wantedGate = null;
    }
  }
  const pick = task ? panelPickOf(task.id) : {};
  // The work view's 離れていた間に, above the summary (my-work.js); a hub has none.
  renderWorkAway(!hub && (task || sess) ? { task, sess, all } : null);
  const shownGate = task && pane !== 'term' ? gateShownIn(task, tabOfPane(pane), all, pick) : null;
  // Opening a tab reads every record judged in it, not only the one shown: the others are earlier
  // rounds behind the round chips, and a 新着 kept for them stays on the tab and the card. Marked
  // by the live record's ref, as isUnread reads it; a copy from the history may lack its board.
  if (task && pane !== 'term') {
    for (const g of all) {
      const r = g.wait === false && paneOfGate(g) === pane ? liveRecordOf(task, g) : null;
      if (r && isUnread(r)) markSeen(gateRef(r));
    }
  }

  setPanelPart('head', tp('tp-head'), hub ? hubPanelHeadHtml(hub, s) : sess ? sessPanelHeadHtml(s) : panelHeadHtml(task));
  const tabs = tp('tp-tabs');
  setPanelPart('tabs', tabs, hub ? hubPanelTabsHtml(hub, s, pane) : sess ? panelTabsHtml(null, s.waiting, s, pane) : panelTabsHtml(task, gate, s, pane, all));
  // The tab chosen is brought into view when the tabs scroll sideways; only when it changed, so a
  // poll does not undo a scroll by hand.
  const tabKey = `${selectedTaskId}/${pane}`;
  if (panelTabFor !== tabKey) {
    panelTabFor = tabKey;
    tabs.querySelector('.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  // Shown before the terminal is mounted: a hidden host has no size to fit to.
  const reveal = pane === 'term' && tp('tp-term').hidden;
  tp('tp-detail').hidden = pane === 'term';
  tp('tp-term').hidden = pane !== 'term';
  syncPanelTerminal(selectedTaskId, s, pane);
  setPanelPart('bar', tp('tp-term-bar'), termBarHtml(s));
  const ph = tp('tp-term-ph');
  ph.hidden = !!panelTerm.term;
  setPanelPart('ph', ph, panelTerm.term ? '' : termPlaceholderHtml(s));
  if (reveal && panelTerm.term) {
    // It was sized while hidden, which it skips; asked again now that it has a size.
    panelTerm.term.fit();
    panelTerm.term.focus();
  }

  // The terminal is all that is shown, so the other tabs are left as they were: what is typed in
  // one is still there when it is shown again, and it is drawn again then.
  if (task && pane === 'term') return renderHandForm(colId === 'backlog' ? task : null);
  if (task) drawTaskPane(task, colId, gate, pane, all, pick, shownGate);
  else drawSessionPane(s, hub, pane);
  renderHandForm(colId === 'backlog' ? task : null);
  tp('tp-form').hidden = !!task && pane !== 'detail';
  // Another task, or another tab, starts at the top, not where the last one was scrolled to; a
  // poll keeps the place. Set once the pane is shown, since a hidden one has no scroll to set.
  const scrollKey = `${selectedTaskId}/${pane}`;
  if (pane !== 'term' && panelScrolledFor !== scrollKey) {
    panelScrolledFor = scrollKey;
    tp('tp-detail').scrollTop = 0;
  }
}

/* The parts of #tp-detail for one of a task's tabs. Its body is what the tab renderers (task-view.js) draw in the
   same tab, from the same functions. */
function drawTaskPane(task, colId, gate, pane, all, pick, shownGate) {
  const key = `${task.id}/${pane}/${shownGate?.id || ''}`;
  const rest = tp('tp-rest');
  // A comment is being typed here: the gate and the body stay as they are until the box is left
  // (the focusout below).
  if (panelCommentFocused() && panelShown?.startsWith(`${task.id}/${pane}/`)) {
    panelHeld = true;
    return;
  }
  panelHeld = false;
  const same = panelShown === key;
  const commentEl = rest.querySelector('.gate-comment');
  const comment = commentEl && same ? commentEl.value : '';
  const openDetails = [...rest.querySelectorAll('details')].map(d => d.open);
  panelShown = key;

  const here = !!gate && paneOfGate(gate) === pane && shownGate?.id === gate.id;
  setPanelPart('links', tp('tp-links'), pane === 'detail' ? ghRowsHtml(task) : '');
  setPanelPart('gate', tp('tp-gate'), panelGateHtml(task, gate, here));
  setPanelPart('rest', rest, panelRestHtml(task, colId, pane, all, pick));
  if (!same) return;
  const box = rest.querySelector('.gate-comment');
  if (box && comment) box.value = comment;
  rest.querySelectorAll('details').forEach((d, i) => { if (openDetails[i] != null) d.open = openDetails[i]; });
}

/* #tp-detail of a hub or of a session with no task: the gate it waits on is judged here, above what it is
   handling. Held, like a task's, while a comment is being typed in the box. */
function drawSessionPane(s, hub, pane) {
  const rest = tp('tp-rest');
  const judge = sessJudgeOf(s, hub);
  const prefix = `${selectedTaskId}/${pane}/`;
  if (panelCommentFocused() && panelShown?.startsWith(prefix)) {
    panelHeld = true;
    return;
  }
  panelHeld = false;
  const key = prefix + (judge.gate?.id || '');
  const same = panelShown === key;
  const commentEl = rest.querySelector('.gate-comment');
  const comment = commentEl && same ? commentEl.value : '';
  const openDetails = [...rest.querySelectorAll('details')].map(d => d.open);
  panelShown = key;
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), judge.banner);
  setPanelPart('rest', rest, judge.html + (hub ? hubDetailHtml(hub, s) : sessDetailHtml(s, pane)));
  if (!same) return;
  const box = rest.querySelector('.gate-comment');
  if (box && comment) box.value = comment;
  rest.querySelectorAll('details').forEach((d, i) => { if (openDetails[i] != null) d.open = openDetails[i]; });
}

/* The open gates a hub or a session with no task waits on, longest first: the ones the hub opened for a person, or the
   ones opened from the worker's worktree. Read from the board's state, so only for a session of the board that is shown. */
function sessGatesOf(s) {
  return (state.gates || [])
    .filter(g => g.wait !== false && (s.kind === 'hub' ? !!g.answeredByHub : !g.answeredByHub && !!s.worktree && g.worktree === s.worktree))
    .sort((a, b) => (stampSecs(a.openedAt) || 0) - (stampSecs(b.openedAt) || 0) || (a.id < b.id ? -1 : 1));
}

/* What the panel of a hub or of a session with no task says of the gates it waits on: the first gate that has no task card
   is judged right here (`html`), the others are named under it, and a gate that has one, or that this page does not read,
   is the banner whose button opens its panel. */
function sessJudgeOf(s, hub) {
  const own = hub ? !hubOther(hub) : boardOfSession(s).own;
  const open = own ? sessGatesOf(s).filter(g => !taskOfGate(g)) : [];
  const banner = s.waiting && !open.some(g => g.id === s.waiting.id) ? sessGateHtml(s) : '';
  if (!open.length) return { gate: null, banner, html: '' };
  const [first, ...more] = open;
  const others = more.length
    ? `<div class="m3-filled-card">${secTitle('ほかの確認待ち')}<ul class="sess-side-list">${more.map(g =>
      `<li><button type="button" class="linkish" data-tp-gate="${esc(gateRef(g))}">${esc(g.title)}</button><span class="who">${esc(kindOf(g.kind)[0])}</span></li>`).join('')}</ul></div>` : '';
  return { gate: first, banner, html: gateJudgeHtml(first, taskForGate(first)) + others };
}

/* A gate with no task card, in the panel: its head and the judge, with no tabs. */
function renderGatePanel(g) {
  tp('tp-tabs').hidden = true;
  tp('tp-detail').hidden = false;
  tp('tp-term').hidden = true;
  // Another subject's terminal is not kept behind this one.
  syncPanelTerminal(selectedTaskId, null, 'detail');
  const rest = tp('tp-rest');
  const key = `${selectedTaskId}/gate/${g.id}`;
  if (panelCommentFocused() && panelShown === key) {
    panelHeld = true;
    return;
  }
  panelHeld = false;
  const same = panelShown === key;
  const commentEl = rest.querySelector('.gate-comment');
  const comment = commentEl && same ? commentEl.value : '';
  const openDetails = [...rest.querySelectorAll('details')].map(d => d.open);
  panelShown = key;
  setPanelPart('head', tp('tp-head'), gatePanelHeadHtml(g));
  setPanelPart('tabs', tp('tp-tabs'), '');
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), '');
  setPanelPart('rest', rest, gateJudgeHtml(g, taskForGate(g)));
  renderHandForm(null);
  tp('tp-form').hidden = true;
  if (same) {
    const box = rest.querySelector('.gate-comment');
    if (box && comment) box.value = comment;
    rest.querySelectorAll('details').forEach((d, i) => { if (openDetails[i] != null) d.open = openDetails[i]; });
  } else {
    tp('tp-detail').scrollTop = 0;
  }
}

function gatePanelHeadHtml(g) {
  const [label] = kindOf(g.kind);
  const b = selectedBoard();
  const origin = b ? boardName(b) : repoName();
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key">確認待ち</span>
        ${origin ? `<span class="origin-chip" title="${esc(b ? `${boardName(b)} (${b.nwo})` : state.repo || '')}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        <span class="m3-pill pill-warn">${esc(label)}</span>
      </div>
      <h2 class="tp-title">${esc(g.title)}</h2>
    </div>
    ${panelBtnsHtml('')}`;
}

/* The clicks of a judge drawn in a panel of a gate, a hub or a session: the buttons that answer, the park button, the IDE and
   the list of other gates. True when it took the click. */
function judgeClick(e) {
  const hit = sel => e.target.closest(sel);
  let b;
  if (parkClick(e)) return true;
  if ((b = hit('[data-gate] [data-act], [data-gate] .pick[data-choice]'))) { decideAct(b); return true; }
  if ((b = hit('[data-ide]'))) { worktreeAct('ide', b.dataset.ide); return true; }
  if ((b = hit('[data-focus]'))) { worktreeAct('focus', b.dataset.focus); return true; }
  if ((b = hit('[data-tp-gate]'))) { goToGate(b.dataset.tpGate); return true; }
  return false;
}

function panelHeadHtml(task) {
  const hcol = humanColOf(task);
  const stuck = stuckOf(task);
  const colObj = COLUMNS.find(c => c.id === columnOf(task));
  const park = parkOf(task);
  const pill = park ? `<span class="m3-pill pill-neutral">置いている — ${esc(parkLabel(park))}</span>`
    : hcol ? `<span class="m3-pill pill-warn">${esc(humanLabel(hcol))}を待っています</span>`
    : stuck ? `<span class="m3-pill pill-err">${esc(stuck)}</span>`
    : `<span class="m3-pill pill-blue">${esc(colObj ? colObj.label : task.status)}</span>`;
  const b = selectedBoard();
  const origin = b ? boardName(b) : (state.repo || '').split('/').pop();
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        ${ghHeadLinksHtml(task)}
        ${origin ? `<span class="origin-chip" title="${esc(b ? `${boardName(b)} (${b.nwo})` : state.repo || '')}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        ${pill}
      </div>
      <h2 class="tp-title">${esc(task.title)}${titlePendingPill(task)}</h2>
    </div>
    ${panelBtnsHtml('<button type="button" class="btn-m3-text tp-jump" data-tp-jump title="エージェントのボードでこのカードを見る">カードへ</button>')}`;
}

/* The head's buttons, which are the same for a task and a hub: `jump` is the one of its own. */
function panelBtnsHtml(jump) {
  const place = (where, icon, label, on) =>
    `<button type="button" class="tool-btn${on ? ' on' : ''}" data-tp-place="${where}" title="${label}" aria-label="${label}" aria-pressed="${on}"><span class="material-symbols-outlined" aria-hidden="true">${icon}</span></button>`;
  // The work view keeps the panel in its right column.
  const placing = view === 'work' ? '' : `${place('left', 'left_panel_open', '左のサイドバーに置く', prefs.panelDock === 'left' && !panelPop())}
      ${place('right', 'right_panel_open', '右のサイドバーに置く', prefs.panelDock === 'right' && !panelPop())}
      ${place('pop', 'open_in_new', 'ダイアログで開く', panelPop())}`;
  return `<div class="tp-head-btns">
      ${jump}
      ${placing}
      <button type="button" class="tool-btn" data-tp-close title="閉じる" aria-label="閉じる"><span class="material-symbols-outlined" aria-hidden="true">close</span></button>
    </div>`;
}

/* `off` is why the tab cannot be used (its tooltip), or falsy. */
const panelTab = (pane, id, label, extra, off) =>
  `<button type="button" role="tab" id="tp-tab-${id}" class="m3-seg-tab${pane === id ? ' active' : ''}" data-pane="${id}" aria-selected="${pane === id}" aria-controls="${id === 'term' ? 'tp-term' : 'tp-detail'}"${off ? ` disabled title="${esc(off)}"` : ''}>${label}${extra}</button>`;

/* A task has five tabs, a session with no task the first two. `all` is the task's gates. */
function panelTabsHtml(task, gate, s, pane, all = []) {
  const usable = hasSession(s) && !!state.boardTerminal?.available;
  const hint = hasSession(s) ? '端末はボードから開けません' : 'セッションなし';
  // The tab shows a word, so five tabs fit; the reason is its tooltip.
  // The work view has the terminal in the middle, beside the panel.
  const term = view === 'work' ? '' : panelTab(pane, 'term', 'ターミナル', !usable ? `<span class="tp-tab-hint">${hasSession(s) ? '端末なし' : hint}</span>` : s?.waiting ? '<span class="tp-wait">入力待ち</span>'
    : s && sessionState(s) === 'permission' ? `<span class="tp-wait">${permissionLabel(s)}</span>` : '', !usable && hint);
  if (!task) return panelTab(pane, 'detail', '詳細', gate ? '<span class="tp-dot" title="あなたの判断待ちがあります"></span>' : '', '') + term;
  // The dot is on the tab the open gate is judged in; the counts and 新着 come from the gates.
  const owner = gate ? paneOfGate(gate) : null;
  const unreadIn = kind => all.some(g => g.kind === kind && g.wait === false && (r => r && isUnread(r))(liveRecordOf(task, g)));
  const tab = (id, label, kind) => {
    const n = kind ? all.filter(g => g.kind === kind).length : 0;
    return panelTab(pane, id, label, (owner === id ? '<span class="tp-dot" title="あなたの判断待ちがあります"></span>' : '')
      + (n ? `<span class="m3-tab-badge" style="background:var(--md-sys-color-surface-container-highest);color:var(--md-sys-color-on-surface);">${n}</span>` : '')
      + (kind && unreadIn(kind) ? '<span class="m3-tab-badge">新着</span>' : ''), '');
  };
  return tab('detail', 'タスクサマリ') + term + tab('review', 'コードレビュー', 'diff') + tab('check', '動作確認', 'verify') + tab('history', '経過');
}

/* ── 詳細 ── */
const secTitle = text => `<div class="tp-sec-title">${text}</div>`;
const kv = (label, value) => `<div class="tp-kv"><span>${label}</span><strong>${value}</strong></div>`;
const monoKv = (label, value, title = '') => `<div class="tp-kv"><span>${label}</span><code title="${esc(title)}">${esc(value)}</code></div>`;

/* The open gate. A decision that needs no comment is one click; reading the plan or the diff,
   and anything that wants a comment, is the judging screen. Not humanActions(): its reply box
   is found by `textarea[data-reply]`, which two on one page would share. `here` is when the tab
   open already shows this gate: its panel is further down, so there is no screen to go to. */
function panelGateHtml(task, gate, here = false) {
  if (!gate) return '';
  const [label] = kindOf(gate.kind);
  const col = gate.humanCol;
  const why = gate.problem || gate.why || '';
  const reasons = stopWhy(gate);
  // The tab's decision panel has the same button and a comment box; this one answers with no
  // comment, so it would drop what was typed there.
  const quick = here ? [] : col === 'dispatch' ? [['start', 'play_arrow']]
    : col === 'plan' || col === 'diff' || col === 'verify' ? [['approve', 'check']]
    : [];
  const btn = ([action, icon]) =>
    `<button type="button" class="btn-m3-primary" data-tp-act="${action}"><span class="material-symbols-outlined" style="font-size:16px;">${icon}</span><span>${esc(actLabel(action, gate.kind))}</span></button>`;
  return `
    <div class="m3-card-attention-box">
      <div class="tp-gate-head">
        <span class="material-symbols-outlined" style="font-size:16px;">pending_actions</span>
        <span>【${esc(label)}】あなたの判断待ち</span>
        <span class="tp-gate-wait">${esc(minutesLabel(waitingMinutes(task)))}待ち</span>
      </div>
      ${parkBannerHtml(task)}
      <div style="font-size:12.5px;">${esc(gate.title)}</div>
      ${why ? `<div style="font-size:11.5px;white-space:pre-wrap;overflow-wrap:anywhere;">${esc(why)}</div>` : ''}
      ${reasons.length ? `<div style="font-size:11.5px;">止めた理由: ${esc(reasons.join(' / '))}</div>` : ''}
      <div class="tp-gate-actions">
        ${quick.map(btn).join('')}
        ${here ? '<span class="tp-muted tp-here"><span class="material-symbols-outlined" style="font-size:14px;" aria-hidden="true">arrow_downward</span><span>判定パネルはこの下にあります</span></span>'
          : `<button type="button" class="${quick.length ? 'btn-m3-tonal' : 'btn-m3-primary'}" data-judge="${esc(gate.id)}"><span class="material-symbols-outlined" style="font-size:16px;">arrow_forward</span><span>判定画面を開く</span></button>`}
        ${col === 'question' && readySessionOfTask(task) ? `<button type="button" class="btn-m3-tonal" data-pane="term"><span class="material-symbols-outlined" style="font-size:16px;">terminal</span><span>ターミナルで答える</span></button>` : ''}
      </div>
    </div>`;
}

const STEPS = [['plan', '計画'], ['implement', '実装'], ['selfreview', 'セルフレビュー'], ['pr', 'PR']];

/* The body of a task's tab: 経過, コードレビュー and 動作確認 are the tab renderers'; タスクサマリ
   is the rest of the card with the 概要 in the middle. */
function panelRestHtml(task, colId, pane, all, pick) {
  if (pane === 'review') return reviewTab(task, all, pick);
  if (pane === 'check') return checkTab(task, all, pick);
  if (pane === 'history') return historyTab(task, all, pick);
  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;
  const at = agentColOf(task);
  // Before the first step nothing is lit; after the last, all of them are.
  const now = at === 'done' ? STEPS.length : STEPS.findIndex(([id]) => id === at);
  const mins = worker?.phase ? phaseMinutes(worker) : null;
  let h = '';

  h += `<div class="m3-filled-card">${secTitle('工程')}
    <ol class="tp-steps">${STEPS.map(([, text], i) => `<li class="${i < now ? 'done' : i === now ? 'now' : ''}">${esc(text)}</li>`).join('')}</ol>
    ${worker?.phase ? `<div class="tp-line">worker は${esc(PHASE_LABEL[worker.phase] || worker.phase)}${worker.present ? '' : '（停止）'}${mins != null ? `（${esc(agoLabel(mins))}から）` : ''}</div>` : ''}
    ${live ? agentFactsHtml(sessionOfTask(task)) : ''}
  </div>`;

  // The overview names the worktree, branch, IDE, 完了条件, 止める所 and the Jules session, so the card does not; its Issue and PR rows and, with the hand-over form above, its 申し送り are left out.
  h += overviewTab(task, all, pick, { panel: true, handForm: colId === 'backlog' });
  // The overview's 問題 is the request itself unless the plan wrote one; then the request is here.
  if (task.body && gateShownIn(task, 'overview', all, pick)?.problem) {
    h += `<div class="m3-filled-card"${expandAttrs('request')}>${expandBtnHtml('依頼内容・プロンプト')}${secTitle('依頼内容・プロンプト')}<p class="tp-text">${esc(task.body)}</p></div>`;
  }

  // Newest first: the one the worker left last is the one that describes where it is now.
  const records = recordsOf(task).reverse();
  h += `<div class="m3-filled-card">${secTitle('記録（止めずに進んだもの）')}
    ${records.length ? records.map(r => {
      const [label] = kindOf(r.kind);
      const [text, tone] = recordSummary(r);
      const pillClass = tone === 'good' ? 'pill-good' : tone === 'bad' ? 'pill-err' : 'pill-warn';
      const icon = tone === 'good' ? 'check_circle' : tone === 'bad' ? 'cancel' : 'info';
      return `<div class="tp-record">
        <span class="m3-pill ${pillClass}"><span class="material-symbols-outlined" style="font-size:12px;margin-right:2px;">${icon}</span><span>${esc(label)}: ${esc(text)}</span></span>
        <div class="tp-muted">${ago(r.openedAt)}に記録</div>
        <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;" data-record="${esc(r.id)}">開く・差し戻す →</button>
      </div>`;
    }).join('') : '<div class="tp-muted">記録はまだありません</div>'}
  </div>`;
  if (task.julesSession && httpUrl(task.pr) && live) h += relayHtml(task);
  return h;
}

/* ── A hub in the panel ──────────────────────────────────────────────────────────────────────
   `selectedTaskId` is `hub:<id>`, and the panel draws the same parts: the hub's session is the
   terminal, and 詳細 is what the hub is handling and what can be done to it. */
/* The page's own hub is the one its state counts; another board's hub is counted by its row in
   the board list. */
const hubOther = h => { const own = pageHub(); return !!own && own.id !== h.id; };
const hubRowOf = h => (multiBoard ? boards.find(b => b.slug === h.slug) : null) || null;
/* Where the hub's own requests go: its board's route when it is not the page's. */
const hubBaseOf = h => multiBoard && hubOther(h) ? `/b/${h.slug}` : BASE;
/* Why this server cannot ask the hub's board, or '': a hub of a board it does not serve. */
const hubAway = h => multiBoard && boards.length > 0 && !!h.slug && !hubRowOf(h) ? 'この hub のボードはこのサーバーにありません' : '';
/* Where the hub's wake goes: its own board's route, null when this server does not have that board. */
const hubPanelWakeBase = h => hubAway(h) ? null : hubBaseOf(h);

/* Why the hub's terminal cannot be opened, as [the tab's hint, its tooltip]; null when it can. */
function hubTermWhy(h, s) {
  if (!s.present) return hubStartingNow(h) ? ['起動中', 'hub を起動しています'] : ['hub 停止中', 'hub が止まっています'];
  if (!state.boardTerminal?.available) return ['端末はボードから開けません', '端末はボードから開けません'];
  if (!boardTerminalReady(s)) return ['tmux の外', 'tmux の外で動いている hub は、ボードから端末を開けません'];
  return null;
}
// The work view has the terminal in the middle, so the panel has no such tab there.
const hubPaneOf = (h, s) => nav.pane === 'term' && view !== 'work' && !hubTermWhy(h, s) ? 'term' : 'detail';

function hubPanelHeadHtml(h, s) {
  const row = hubRowOf(h);
  const since = sinceLabel(row?.hubLastAlive);
  const [pillText, pillCls] = hubRestartPending(h) ? ['再起動しています…', 'pill-neutral']
    : hubStartingNow(h) ? ['起動しています…', 'pill-neutral']
    : !s.present ? [`停止中${since ? ` · ${since}` : ''}`, 'pill-err']
    : s.waiting ? ['入力待ち', 'pill-warn']
    : sessionState(s) === 'permission' ? [permissionLabel(s), 'pill-warn']
    : sessionState(s) === 'unknown' ? [STATE_LABEL.unknown, STATE_PILL.unknown]
    : ['稼働中', 'pill-good'];
  const origin = row ? boardName(row) : repoName();
  const nwo = row?.nwo || state.repo || '';
  const title = h.parent ? `親タスク hub — ${[h.key, h.title].filter(Boolean).join(' ') || '（キー不明）'}`
    : `リポジトリ hub — ${nwo}`;
  const jump = multiBoard && hubOther(h)
    ? '<button type="button" class="btn-m3-text tp-jump" data-tp-board title="この hub のボードを開く">ボードへ</button>' : '';
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(h.id)}">hub</span>
        ${origin ? `<span class="origin-chip" title="${esc(nwo)}"><span class="material-symbols-outlined" aria-hidden="true">${h.parent ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        <span class="m3-pill ${pillCls}">${esc(pillText)}</span>
      </div>
      <h2 class="tp-title">${esc(title)}</h2>
    </div>
    ${panelBtnsHtml(jump)}`;
}

function hubPanelTabsHtml(h, s, pane) {
  const why = hubTermWhy(h, s);
  if (view === 'work') return panelTab(pane, 'detail', '詳細', '', '');
  return panelTab(pane, 'detail', '詳細', '', '')
    + panelTab(pane, 'term', 'ターミナル', why ? `<span class="tp-tab-hint">${esc(why[0])}</span>` : s.waiting ? '<span class="tp-wait">入力待ち</span>'
      : sessionState(s) === 'permission' ? `<span class="tp-wait">${permissionLabel(s)}</span>` : '', why && why[1]);
}

const HUB_LIST_MAX = 5;
const hubListMore = n => n > 0 ? `<div class="tp-muted">ほか ${n} 件</div>` : '';

/* The worker sessions of the board shown that have no process: a worktree with no session, or one whose session ended. The
   repository hub's lists the whole board, grouped by the hub each belongs to; a parent-task hub's, its own. */
function idleWorktreesOf(h) {
  const repoHub = repoHubId();
  const groups = new Map();
  for (const s of state.sessions || []) {
    if (s.kind !== 'worker') continue;
    const st = sessionState(s);
    if (st !== 'none' && st !== 'ended') continue;
    const owner = hubOfSession(s) || repoHub;
    if (h.parent && owner !== h.id) continue;
    groups.set(owner, [...(groups.get(owner) || []), s]);
  }
  return [...groups].map(([id, list]) => {
    const hub = (state.hubs || []).find(x => x.id === id);
    return { id, label: hub ? hubShortName(hub) : id, repo: !hub?.parent, list: list.sort((a, b) => sessionKey(a) < sessionKey(b) ? -1 : sessionKey(a) > sessionKey(b) ? 1 : 0) };
  }).sort((a, b) => (b.repo - a.repo) || (a.label < b.label ? -1 : a.label > b.label ? 1 : 0));
}

/* 「動いていない worktree（N）」: each one opens as a session of its own, where it can be resumed, opened in the IDE or cleaned up. */
function idleWorktreesHtml(h) {
  // Another board's hub has the sessions of its own board, which this page does not read.
  if (hubOther(h) && !h.parent) return '';
  const groups = idleWorktreesOf(h);
  const n = groups.reduce((sum, g) => sum + g.list.length, 0);
  if (!n) return '';
  const many = groups.length > 1;
  const items = g => `<ul class="sess-side-list">${g.list.map(s => `<li><button type="button" class="linkish" data-tp-worktree="${esc(s.id)}">${esc(sessionKey(s))}</button><span class="who">${esc(s.branch || STATE_LABEL[sessionState(s)])}</span></li>`).join('')}</ul>`;
  return `<div class="m3-filled-card"><details class="tp-idle"><summary>動いていない worktree（${n}）</summary>${groups.map(g => (many ? `<div class="tp-muted">${esc(g.label)}</div>` : '') + items(g)).join('')}</details></div>`;
}

/* A worktree of that card, opened as a session: in 「いまの仕事」 where the person is in it. */
function openIdleWorktree(id) {
  if (view === 'work') return go({ board: nav.board, view: 'work', task: SESS_REF + id, pane: 'detail' });
  openTaskPanel(SESS_REF + id, 'detail');
}

function hubDetailHtml(h, s) {
  const other = hubOther(h);
  const row = hubRowOf(h);
  const startWhy = hubStartWhy(h) || hubWaitWhy(h);
  let html = '';
  if (!s.present) {
    const since = sinceLabel(row?.hubLastAlive);
    html += `<div class="m3-card-attention-box">
      <div class="tp-gate-head"><span class="material-symbols-outlined" style="font-size:16px;">stop_circle</span><span>hub は${since ? `${esc(since)}から` : ''}止まっています。起動するとここでターミナルを開けます。</span></div>
      <div class="tp-gate-actions"><button type="button" class="btn-m3-primary" data-tp-hub="start"${startWhy ? ' disabled' : ''} title="${esc(startWhy || 'tmux の新しいウィンドウで adj hub を実行します')}"><span class="material-symbols-outlined" style="font-size:16px;">play_arrow</span><span>hub を起動</span></button></div>
    </div>`;
  }

  // The sessions asked of this hub that it has not started yet.
  const pend = sessionPendingRows().filter(p => p.hubId === h.id);
  if (pend.length) html += `<div class="m3-filled-card">${secTitle('起動を依頼中')}${pend.map(pendingRowHtml).join('')}</div>`;

  // The page's own board is read from its state; another board's from its row in the list.
  const line = other ? [] : (state.tasks || []).filter(t => t.status === 'queued').sort((a, b) => (a.order || 0) - (b.order || 0));
  const queued = other ? row?.queued : line.length;
  const counts = [];
  if (!other && state.workerSlots) counts.push(kv('worker', `${state.workerSlots.busy} / ${state.workerSlots.max} 稼働`));
  else if (other && row) counts.push(kv('作業中', `${row.working || 0} 件`));
  if (!other) counts.push(kv('あなたの確認待ち', `${waitingIn()} 件`));
  else if (row) counts.push(kv('あなたの確認待ち', `${row.waiting || 0} 件`));
  if (queued != null) counts.push(kv('待ちキュー', `${queued} 件`));
  counts.push(kv('受信箱', `${h.inboxCount || 0} 件`));
  html += `<div class="m3-filled-card">${secTitle('いまの状態')}<div class="tp-kvs">${counts.join('')}</div>${agentFactsHtml(s)}</div>`;

  // Only the page's own board has its tasks to name; another board's are read on its own page.
  if (!other) {
    html += `<div class="m3-filled-card">${secTitle('待ちキュー')}${line.length
      ? `<ul class="sess-side-list">${line.slice(0, HUB_LIST_MAX).map(t => `<li><button type="button" class="linkish" data-tp-task="${esc(t.id)}">${esc(t.title)}</button><span class="who">${esc(t.id)}</span></li>`).join('')}</ul>${hubListMore(line.length - HUB_LIST_MAX)}`
      : '<div class="tp-muted">待ちはありません</div>'}</div>`;
  }

  // The hub reports the newest first and `inboxCount` is the whole of it. What is still to be read
  // comes first, oldest first, so the message the board's entry names the age of is in view.
  const unread = m => m.counted && !m.seen;
  const wakeOff = hubWakeBlocked(h, hubPanelWakeBase(h));
  const wakeWhy = hubWakeWhy(h);
  const wake = (h.unseen || 0) + (h.seen || 0) > 0
    ? `<div class="tp-hub-wake"><button type="button" class="btn-m3-tonal" data-tp-hub="wake" data-wake-slug="${esc(h.slug)}"${wakeOff ? ' disabled' : ''} title="${esc(wakeOff || HUB_WAKE_TITLE)}"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">notifications_active</span><span>hub を起こす</span></button><span class="hub-wake-why" role="status">${esc(wakeWhy)}</span></div>` : '';
  const listed = h.inbox || [];
  const inbox = [...listed.filter(unread).reverse(), ...listed.filter(m => !unread(m))].slice(0, HUB_LIST_MAX);
  html += `<div class="m3-filled-card">${secTitle('受信箱')}${inbox.length
    ? `<ul class="sess-side-list">${inbox.map(m => `<li><div>${unread(m) ? '<span class="m3-pill pill-warn">未確認</span> ' : ''}${esc(m.subject || m.name)}</div><div class="who">${esc([m.kind, m.from, m.at ? when(m.at) : ''].filter(Boolean).join(' ・ '))}</div></li>`).join('')}</ul>${hubListMore((h.inboxCount || 0) - inbox.length)}`
    : '<div class="tp-muted">受信箱は空です</div>'}${wake}</div>`;

  // A stopped hub's start is the banner's.
  const act = hubActionOf(s);
  const own = act && act.act !== 'hub-start'
    ? `<button type="button" class="btn-m3-tonal" data-tp-hub="${act.act.slice(4)}"${act.disabled ? ' disabled' : ''} title="${esc(act.title)}"><span class="material-symbols-outlined" style="font-size:16px;">${act.icon}</span><span>${esc(act.label)}</span></button>` : '';
  const restartBtn = canRestart(s) ? (() => {
    const why = restartWhy(s);
    return `<button type="button" class="btn-m3-tonal" data-tp-hub="restart"${why ? ' disabled' : ''} title="${esc(why || '今の会話のまま、止めて起動し直します（adj hub --resume）')}"><span class="material-symbols-outlined" style="font-size:16px;">autorenew</span><span>セッションを再起動…</span></button>`;
  })() : '';
  html += idleWorktreesHtml(h);

  // Starting a session with no task, or a parent task's hub, asks this page's board: not another one's, nor one this server
  // does not serve.
  const away = hubAway(h);
  const startOff = !state.resident ? 'resident サーバーのボードからだけ使えます' : away || (!state.sessionStart?.agent ? '起動するエージェントが分かりません' : '');
  const parentOff = !state.resident ? 'resident サーバーのボードからだけ使えます' : away || (!state.hubStart?.available ? NO_START : '');
  const addBtn = (act, icon, label, off, title) =>
    `<button type="button" class="btn-m3-tonal" data-tp-hub="${act}"${off ? ' disabled' : ''} title="${esc(off || title)}"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">${icon}</span><span>${label}</span></button>`;
  html += `<div class="m3-filled-card">${secTitle('操作')}
    <div class="tp-gate-actions">
      <button type="button" class="btn-m3-tonal" data-tp-hub="next" title="adj send --kind next (着手を促す)"><span class="material-symbols-outlined" style="font-size:16px;">bolt</span><span>着手を促す</span></button>
      <button type="button" class="btn-m3-tonal" data-tp-hub="sync" title="再同期 (adj refresh)"><span class="material-symbols-outlined" style="font-size:16px;">refresh</span><span>再同期</span></button>
      ${own}
      ${restartBtn}
    </div>
    <div class="tp-gate-actions">
      ${addBtn('add-session', 'terminal', 'タスクなしのセッションを始める…', startOff, 'この hub にタスクなしのセッションを頼みます')}
      ${h.parent ? '' : addBtn('add-parent-hub', 'account_tree', '親タスクの hub を起動…', parentOff, '親タスクのキーを入れて、その hub を tmux の新しいウィンドウで起動します')}
    </div>
    <button type="button" class="btn-m3-text tp-reset" data-tp-hub="reset"${startWhy ? ' disabled' : ''} title="${esc(startWhy || 'hub をリセット：新しい会話で hub を起動し直します（adj hub --new）')}">hub をリセット…</button>
  </div>`;
  return html;
}

/* ── A session with no task in the panel ────────────────────────────────────────────────────
   `selectedTaskId` is `session:<id>`: the session is the terminal, and 詳細 is what the Sessions
   list used to show beside it (sessDetailHtml). Once it is linked to a task the panel shows that. */
function sessPanelHeadHtml(s) {
  const st = sessionState(s);
  const label = sessionLabel(s, true);
  const b = boardOfSession(s);
  const hubName = b.hub ? hubLabel(b.hub) : `${b.hubId}（一覧にありません）`;
  const what = s.task ? 'タスクが見つからない' : 'タスクなし';
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(s.id)}">${esc(label.tag || 'セッション')}</span>
        ${hubName ? `<span class="origin-chip" title="${esc(hubName)}"><span class="material-symbols-outlined" aria-hidden="true">${b.hub?.parent ? 'account_tree' : 'folder'}</span><span>${esc(hubName)}</span></span>` : ''}
        <span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(STATE_LABEL[st])}</span>
        <span class="m3-pill pill-neutral">${esc(what)}</span>
      </div>
      <h2 class="tp-title" title="${esc(s.branch ? `ブランチ: ${s.branch}` : '')}">${esc(label.text)}</h2>
    </div>
    ${panelBtnsHtml('')}`;
}

/* ── ターミナル ── */
/* The terminal lives in #tp-term-host from its first mount until the task changes or the panel
   closes. The other tabs only hide the pane around it, so the socket survives; the bar and the note
   over it are the parts that are drawn again. */
function syncPanelTerminal(subject, s, pane) { syncTermSlot(panelTerm, subject, s, pane); }
function disposePanelTerminal() { disposeTermSlot(panelTerm); }

/* A slot is one terminal's place: where it mounts (`host`), what to draw again when it ends
   (`redraw`) and which board's path it connects through (`base`). The task panel has one and the
   work view's middle another, and both keep the socket across a redraw the same way. */
function syncTermSlot(slot, subject, s, pane) {
  // Another task's (or hub's) socket is not carried over; a fresh one is asked for after
  // 再開 or 再接続. A session that was linked to a task keeps its own: only the subject's name
  // changed.
  const sameSession = !!slot.term && !!s && s.id === slot.sessionId;
  if ((slot.taskId !== subject && !sameSession) || (slot.term && s && s.id !== slot.sessionId)
      || (slot.reconnect && boardTerminalReady(s))) disposeTermSlot(slot);
  slot.taskId = subject;
  if (pane !== 'term' || slot.term || !boardTerminalReady(s)) return;
  slot.sessionId = s.id;
  slot.ended = null;
  const handle = mountSessionTerminal(slot.host(), {
    sessionId: s.id,
    base: slot.base(),
    focus: slot.focus !== false,
    onEnd: code => {
      if (slot.term !== handle) return;
      slot.ended = code;
      keepScreen({ sessionId: s.id, term: handle });
      slot.redraw();
    },
  });
  slot.term = handle;
}

function disposeTermSlot(slot) {
  const { term, sessionId } = slot;
  if (term) {
    keepScreen({ sessionId, term });
    term.dispose();
  }
  Object.assign(slot, { taskId: null, sessionId: null, term: null, ended: null, reconnect: false });
}

function termBarHtml(s, slot = panelTerm, actions = true) {
  if (!s || !hasSession(s)) return '';
  const st = sessionState(s);
  const last = s.present ? lastOutputText(s) : null;
  const again = slot.term && slot.ended != null && boardTerminalReady(s);
  const reset = actions ? hubResetButton(s) : null;
  return `<span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(STATE_LABEL[st])}</span>`
    + (last ? `<span class="tp-muted">最後の出力: ${esc(last)}</span>` : '')
    + `<span class="tp-bar-gap"></span>`
    + (reset ? actionButtonHtml(reset) : '')
    + (again ? '<button type="button" class="btn-m3-tonal sess-act" data-tp-reconnect><span class="material-symbols-outlined" aria-hidden="true">sync</span><span>再接続</span></button>' : '')
    + (actions ? sessionButtons(s).bar.map(b => actionButtonHtml(b)).join('') : '');
}

/* Over the host while there is no terminal in it: why not, and the last screen if this page saw
   one. */
function termPlaceholderHtml(s) {
  if (!hasSession(s)) {
    return `<div class="tp-ph-head">セッションはありません</div>${s?.worktree ? `<div class="tp-muted">${esc(s.worktree)}${s.branch ? `（${esc(s.branch)}）` : ''}</div>` : ''}`;
  }
  if (boardTerminalReady(s)) return '<div class="tp-muted">接続しています…</div>';
  const st = restingState(s);
  const head = st === 'stopped' ? 'セッションは止まっています' : st === 'ended' ? 'セッションは終了しています'
    : !s.present ? 'このセッションは動いていません'
    : 'このセッションの端末はボードから開けません（tmux で動いているセッションだけ開けます）';
  const shot = sessView.screens[s.id];
  const mins = shot && state.now != null && shot.at != null ? minutesSince(shot.at, state.now) : null;
  const resume = canResume(s) ? '' : !s.present && s.kind === 'worker'
    ? `<div class="tp-muted">${esc(!s.conversation ? '保存された会話がないため再開できません' : state.sessionResume?.reason || 'ボードからは再開できません')}</div>` : '';
  return `<div class="tp-ph-head">${esc(head)}</div>`
    + (shot?.lines.length ? `<div class="tp-muted">このページで最後に見た画面${mins == null ? '' : `（${esc(agoLabel(mins))}）`}</div><pre class="sess-last-out">${esc(shot.lines.join('\n'))}</pre>` : '')
    + resume;
}

/* Events are heard on the panel itself: its parts are drawn again, it is not. */
tp('task-panel').addEventListener('click', e => {
  const hub = hubOfRef(selectedTaskId);
  const task = hub || isHubRef(selectedTaskId) ? null : taskById(selectedTaskId);
  const sess = !hub && !task ? sessOfRef(selectedTaskId) : null;
  if (!task && !hub && !sess) {
    if (gateOfPanelRef(selectedTaskId)) return gatePanelClick(e);
    return view === 'work' ? workPanelClick(e) : undefined;
  }
  const hit = sel => e.target.closest(sel);
  let b;
  if ((b = hit('[data-pane]'))) {
    // 「ターミナルで答える」 in the work view moves to the middle terminal, and the tab stays.
    if (view === 'work' && b.dataset.pane === 'term') return focusWorkTerm();
    if (!b.disabled) go({ pane: b.dataset.pane }, { replace: true });
    return;
  }
  if (hit('[data-tp-close]')) {
    return closeTaskPanel();
  }
  if ((b = hit('[data-tp-place]'))) return placePanel(b.dataset.tpPlace);
  if (hub) return hubPanelClick(e, hub);
  if (sess) return sessPanelClick(e, sess);
  if (hit('[data-tp-jump]')) {
    // A dialog covers the card, so it closes first; a docked panel stays beside it.
    const id = task.id;
    if (panelPop()) closeTaskPanel(); else renderTaskPanel();
    return jump('agent', id);
  }
  if (parkClick(e)) return;
  if ((b = hit('[data-tp-act]'))) return act(b.dataset.tpAct, task.id);
  // The decision panel and a gate's choices, which `bindDecide` wires in the other views; not
  // here, since it would bind [data-ide] a second time next to the one below.
  if ((b = hit('[data-gate] [data-act], [data-gate] .pick[data-choice]'))) return decideAct(b);
  // Links to a gate or a record show it in the tab it is read in, not on a page of their own.
  if ((b = hit('[data-record]'))) {
    const r = recordById(b.dataset.record);
    return r ? showPanelTab(task, paneOfGate(r), r.id) : goToGate(b.dataset.record);
  }
  if ((b = hit('[data-judge]'))) {
    const g = gatesOf(task).find(x => x.id === b.dataset.judge);
    return g && showPanelTab(task, paneOfGate(g), g.id);
  }
  // A gate in 経過 opens in the tab of its kind, rather than being repeated here.
  if ((b = hit('[data-open]'))) {
    const g = gatesOf(task).find(x => x.id === b.dataset.open);
    return g && showPanelTab(task, paneOfGate(g), g.id);
  }
  if ((b = hit('[data-pick]'))) {
    panelPickOf(task.id)[tabOfPane(paneOf(task))] = b.dataset.pick;
    return renderTaskPanel();
  }
  if (hit('[data-unpick]')) {
    delete panelPickOf(task.id)[tabOfPane(paneOf(task))];
    return renderTaskPanel();
  }
  if ((b = hit('[data-focus]'))) return worktreeAct('focus', b.dataset.focus);
  if ((b = hit('[data-add-child]'))) return openChildForm(b.dataset.addChild, b.dataset.addChildBoard);
  // The button is what is disabled while it asks, not the panel this handler is on.
  if ((b = hit('[data-fetch-issue]'))) return fetchIssue(b.dataset.fetchIssue, { currentTarget: b });
  if ((b = hit('[data-ide]'))) return worktreeAct('ide', b.dataset.ide);
  if ((b = hit('[data-findings]'))) return loadFindings(b.dataset.findings);
  if ((b = hit('[data-relay]'))) return relayPicked(b.dataset.relay);
  if ((b = hit('[data-sess-act]'))) {
    const s = sessionOfTask(task);
    if (!s || b.disabled) return;
    // A session resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, s);
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
});
/* A gate with no task card: its head's buttons and its judge. */
function gatePanelClick(e) {
  let b;
  if (e.target.closest('[data-tp-close]')) return closeTaskPanel();
  if ((b = e.target.closest('[data-tp-place]'))) return placePanel(b.dataset.tpPlace);
  judgeClick(e);
}
/* Another tab of the open task, with `gateId` picked in it when given. */
function showPanelTab(task, pane, gateId) {
  if (gateId) {
    panelPickOf(task.id)[tabOfPane(pane)] = gateId;
    // A gate picked is read from its top.
    panelScrolledFor = null;
  }
  if (nav.pane === pane) renderTaskPanel(); else go({ pane }, { replace: true });
}

/* A session with no task: its link and git buttons, its gate, and the terminal bar's (the
   session's own `runSessionAction`, as for a task). */
function sessPanelClick(e, s) {
  const hit = sel => e.target.closest(sel);
  let b;
  if ((b = hit('[data-side-act]'))) {
    if (b.disabled) return;
    const act = b.dataset.sideAct;
    if (act === 'git-refresh') {
      ensureGit(s, true);
      return renderTaskPanel();
    }
    if (act === 'link-new' || act === 'link-existing') return openLinkDialog(s, act === 'link-new' ? 'new' : 'existing');
    if (act === 'goto-board' && s.task) return go({ board: b.dataset.board, view: 'work', task: s.task, pane: 'detail' });
    return;
  }
  if (hit('[data-tp-sess-gate]') && s.waiting) return goToGate(s.waiting.id, s.waiting.slug);
  if (judgeClick(e)) return;
  if ((b = hit('[data-sess-act]'))) {
    if (b.disabled) return;
    // A session resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, s);
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
}
/* The hub's own buttons. The terminal bar's are the session's (`runSessionAction`), as for a task. */
function hubPanelClick(e, h) {
  const hit = sel => e.target.closest(sel);
  let b;
  const waiting = hubSessionOf(h).waiting;
  if (hit('[data-tp-sess-gate]') && waiting) return goToGate(waiting.id, waiting.slug);
  if (judgeClick(e)) return;
  if ((b = hit('[data-tp-hub]'))) {
    if (b.disabled) return;
    switch (b.dataset.tpHub) {
      case 'next': return nudgeHub(hubBaseOf(h));
      case 'sync': return refreshAll(hubBaseOf(h));
      case 'start': return hubStart(h.id);
      case 'stop': return openHubStopDialog(h.id, 'stop');
      case 'close': return openHubStopDialog(h.id, 'close');
      case 'reset': return openHubStopDialog(h.id, 'reset');
      case 'restart': return openHubStopDialog(h.id, 'restart');
      case 'wake': return wakeHub(h, hubPanelWakeBase(h));
      case 'add-session': return openStartDialog();
      case 'add-parent-hub': return openHubKeyDialog();
    }
    return;
  }
  if ((b = hit('[data-pend-act]'))) return pendClick(b);
  if ((b = hit('[data-tp-worktree]'))) return openIdleWorktree(b.dataset.tpWorktree);
  if ((b = hit('[data-tp-task]'))) return openTaskPanel(b.dataset.tpTask);
  if (hit('[data-tp-board]')) {
    // The hub's board, with the hub still in the panel on the tab it was on; a dialog would cover
    // the board, so it closes instead.
    if (panelPop()) {
      hideTaskPanelState();
      return go({ board: h.slug, task: null, pane: 'detail' });
    }
    return go({ board: h.slug, task: selectedTaskId, pane: nav.pane });
  }
  if ((b = hit('[data-sess-act]'))) {
    if (b.disabled) return;
    // A hub started or resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume' || b.dataset.sessAct === 'hub-start') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, hubSessionOf(h));
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
}
tp('task-panel').addEventListener('focusout', e => {
  if (!panelHeld || !e.target.matches('.gate-comment')) return;
  // Focus moving to a button of the panel: drawing now would replace the button between its
  // mousedown and its click and lose the click. That click (or the next poll) redraws instead.
  if (e.relatedTarget && tp('task-panel').contains(e.relatedTarget)) return;
  setTimeout(() => {
    if (!panelHeld || panelCommentFocused()) return;
    panelHeld = false;
    renderTaskPanel();
  });
});
tp('task-panel').addEventListener('change', e => {
  const b = e.target.closest('[data-relay-pick]');
  if (!b) return;
  if (b.checked) relay.picked.add(b.dataset.relayPick); else relay.picked.delete(b.dataset.relayPick);
  renderTaskPanel();
});
// Clicking outside the dialog closes the panel; the dialog mode stays.
tp('tp-scrim').addEventListener('click', () => closeTaskPanel());

/* The width is dragged from the edge that faces the page, saved when the pointer is let go;
   the arrow keys move it a step at a time. */
function setPanelWidth(raw) {
  const rail = tp('nav-rail').offsetWidth;
  prefs.panelWidth = Math.round(Math.max(320, Math.min(raw, innerWidth - rail - 320)));
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
}
// A double click puts the width back to where it started.
tp('tp-resize').addEventListener('dblclick', () => {
  if (panelPop()) return;
  prefs.panelWidth = 520;
  document.body.style.setProperty('--panel-w', '520px');
  savePrefs();
});
tp('tp-resize').addEventListener('keydown', e => {
  if ((e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') || panelPop() || matchMedia('(max-width: 720px)').matches) return;
  e.preventDefault();
  // The panel grows away from its side: toward the right on the left, toward the left on the right.
  const grow = (e.key === 'ArrowRight') === (prefs.panelDock !== 'right');
  setPanelWidth(tp('task-panel').offsetWidth + (grow ? 24 : -24));
  savePrefs();
});
tp('tp-resize').addEventListener('pointerdown', e => {
  if (panelPop() || matchMedia('(max-width: 720px)').matches) return;
  e.preventDefault();
  const handle = e.currentTarget;
  handle.setPointerCapture(e.pointerId);
  document.body.classList.add('tp-dragging');
  const move = ev => {
    const rail = tp('nav-rail').offsetWidth;
    setPanelWidth(prefs.panelDock === 'right' ? innerWidth - ev.clientX : ev.clientX - rail);
  };
  const end = () => {
    handle.removeEventListener('pointermove', move);
    handle.removeEventListener('pointerup', end);
    handle.removeEventListener('pointercancel', end);
    document.body.classList.remove('tp-dragging');
    savePrefs();
  };
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
});

/* The hand-over form of a backlog task. The board redraws once a minute and on every change of
   state, and a textarea built again loses what is typed into it, the caret, and an IME
   composition in progress. So the form is built again only for another task, or when the saved
   instruction changed while the box is neither typed in nor focused. When it changed while the
   box is being written, the box is kept and a note says so, with a button to load the new one. */
function renderHandForm(task) {
  const el = document.getElementById('tp-form');
  if (!el) return;
  if (!task) {
    el.replaceChildren();
    delete el.dataset.task;
    return;
  }
  const saved = task.instruction || '';
  const kept = el.querySelector('#tp-instruction');
  if (kept && el.dataset.task === task.id) {
    // Compared with what the box showed, not the saved string: the parser drops a leading newline
    // and turns CRLF into LF, so the saved string can differ from an untouched box.
    const writing = kept.value !== el.dataset.shown || document.activeElement === kept;
    if (el.dataset.saved === saved || writing) {
      el.querySelector('[data-stale]').hidden = el.dataset.saved === saved;
      return;
    }
  }
  el.dataset.task = task.id;
  el.dataset.saved = saved;
  el.innerHTML = `
      <div class="m3-filled-card" style="display:flex;flex-direction:column;gap:8px;">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;">キューへの受け渡し</div>
        <label for="tp-instruction" style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);">エージェントへの申し送り（指示）</label>
        <div data-stale hidden style="font-size:11.5px;color:var(--md-sys-color-error);">
          保存済みの申し送りが別の所で変わりました。いまの入力のまま渡すと上書きします。
          <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;" data-reload>変わった内容を読み込む</button>
        </div>
        <textarea id="tp-instruction" placeholder="追加の指示や申し送りがあれば入力（任意）..." style="width:100%;box-sizing:border-box;border-radius:var(--md-shape-corner-xs);border:1px solid var(--md-sys-color-outline-variant);padding:8px 10px;background:var(--md-sys-color-surface-container-high);color:var(--md-sys-color-on-surface);font-size:12.5px;font-family:inherit;resize:vertical;min-height:60px;">${esc(saved)}</textarea>
        <button class="btn-m3-primary" style="width:100%" data-tp-hand="${esc(task.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">send</span>
          <span>待機キューに渡す</span>
        </button>
      </div>
    `;
  const textarea = el.querySelector('#tp-instruction');
  el.dataset.shown = textarea.value;
  el.querySelector('[data-reload]').addEventListener('click', () => {
    const now = (state.tasks || []).find(t => t.id === task.id);
    if (!now) return;
    el.replaceChildren();
    renderHandForm(now);
  });
  el.querySelector('[data-tp-hand]').addEventListener('click', () =>
    hand(task.id, textarea.value.trim()));
  textarea.addEventListener('keydown', (e) => {
    // keyCode 229 too: where compositionend comes first, the keydown that confirms the IME
    // text already has isComposing false.
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      hand(task.id, textarea.value.trim());
    }
  });
}

registerView('task-panel', {
  render: () => renderTaskPanel(),
  reset: () => {
    hideTaskPanelState();
    // Keyed by a bare task id: they belong to the board being left.
    for (const k of Object.keys(histories)) delete histories[k];
    historyFailed.clear();
  },
});
