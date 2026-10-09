/* ── Where the page is: one board, every board, or the review queue ──────────────────────
   Navigation state lives in the address, so back/forward and a pasted link land on the same
   screen without a reload; what is only a preference (layout, folded repositories) does not.
     /b/<slug>/?view=human&task=<id>&pane=term             one board; pane is detail (the default),
                                                           term, review, check or history
     /b/<slug>/?task=hub:<id>&pane=term                    one board, a hub in the panel
     /b/<slug>/?view=sessions&task=session:<id>&pane=term  its sessions, one with no task in the panel
     /                                                     すべて, every board
     /review?item=<id>                                     要対応, every board
     /?view=work                                           いまの仕事, every board; nothing selected
     /b/<slug>/?view=work&task=<ref>&pane=detail           いまの仕事 with a row selected: the board of
                                                           the row's task, hub or parent (<ref> may also
                                                           be parent:<key>)
   A board served on its own has no list of boards, so it is `board: null` at `/`. Every
   address carries `?token=`: the server refuses a GET without it. */
/* The task panel's tabs, as `pane=` names them; anything else is 詳細 (タスクサマリ). */
const PANES = ['detail', 'term', 'review', 'check', 'history'];
const nav = { board: null, view: 'agent', task: null, pane: 'detail', item: null };
let boards = [];                 // /api/boards: the sidebar's rows
let multiBoard = /^\/b\//.test(location.pathname);   // the resident server: more than one board
let navEpoch = 0;                // bumped on a board switch, so a late answer for the old one is dropped
let navApplying = false;
let pendingTask = null;          // opened once the board's first state is in
let pendingGate = null;          // `<board>/<gate>` of an old link, opened once the work document is in (my-work.js)
let boardJob = null;             // { slug, fn }: run once that board has loaded
const boardStates = {};          // 「すべて」: each board's last state, by slug
const baseOf = x => x?._base || BASE;
/* The board a review item is on: the part of its ref before the `/`. */
const slugOfRef = ref => { const i = String(ref || '').indexOf('/'); return i > 0 ? ref.slice(0, i) : null; };
/* 「すべて」 is the merged board. The work view has no board of its own while nothing is selected, and
   reads its own document instead (my-work.js): it is not 「すべて」, whatever `nav.board` says. */
const scopeAll = () => multiBoard && nav.board === 'all' && nav.view !== 'work';
/* No board is chosen: 「すべて」, or the work view with nothing selected. What would be sent to a board is asked for one. */
const noBoard = () => multiBoard && (!nav.board || nav.board === 'all');
/* The view `/` opens on a server that serves several boards. A constant so that one change moves it. */
const DEFAULT_MULTI_VIEW = 'agent';

function parseUrl(loc = location) {
  const q = new URLSearchParams(loc.search);
  const out = { board: null, view: 'agent', task: q.get('task'), pane: PANES.includes(q.get('pane')) ? q.get('pane') : 'detail', item: q.get('item') };
  const m = /^\/b\/([^/]+)/.exec(loc.pathname);
  out.board = m ? m[1] : multiBoard ? 'all' : null;
  // `/` on a server with several boards opens the view its constant names; a board's own address, the one before.
  if (multiBoard && out.board === 'all') out.view = DEFAULT_MULTI_VIEW;
  const v = q.get('view');
  // `view=agent`, from before エージェント became the default, lands here by falling through.
  if (v === 'human') out.view = v;
  // The work view reads every board of a resident server: a board served alone has none.
  if (v === 'work' && multiBoard) out.view = 'work';
  // The addresses of the セッション tab and of 要対応, which are gone, land where nav-legacy.js says.
  const legacy = legacyNav(loc, multiBoard);
  if (legacy) Object.assign(out, legacy.nav, { item: null });
  // The task panel opens on a board of its own; 「すべて」 switches to the card's board first.
  if (out.board === 'all') out.task = null;
  return out;
}

function urlOf(n = nav) {
  const path = n.view === 'review' ? '/review' : n.board && n.board !== 'all' ? `/b/${n.board}/` : '/';
  let url = path + '?token=' + encodeURIComponent(TOKEN);
  if (n.view === 'human' || n.view === 'sessions' || n.view === 'work') url += `&view=${n.view}`;
  if (n.task && n.view !== 'review') url += `&task=${encodeURIComponent(n.task)}`;
  if (n.task && n.view !== 'review' && n.pane !== 'detail') url += `&pane=${n.pane}`;
  if (n.item && n.view === 'review') url += `&item=${encodeURIComponent(n.item)}`;
  return url;
}

/* A change of address that is not a move: the card being looked at, the gate being read. */
function setNav(patch) {
  Object.assign(nav, patch);
  history.replaceState(null, '', urlOf());
}

const sameNav = (a, b) => a.board === b.board && a.view === b.view && a.task === b.task && a.item === b.item && a.pane === b.pane;

/* Everything that pointed into the board being left. */
function switchBoard() {
  navEpoch++;
  BASE = multiBoard && nav.board && nav.board !== 'all' ? `/b/${nav.board}` : '';
  lastStateJson = '';
  lastMinute = null;
  if (!multiBoard) seenGateIds = null;
  // Each view forgets what it kept for the board being left.
  resetViews();
  boardJob = null;
  // 「すべて」 shows what it last read while the new round is on its way; the review queue
  // keeps the gate it was asked for until that round is in (see renderReview).
  allRound = false;
  state = scopeAll() && Object.keys(boardStates).length ? mergeStates(readBoards(), Math.floor(Date.now() / 1000)) : emptyState();
}

/* From the work view to 「すべて」: the board did not change, but the state was emptied while the work view
   was on screen, so it is drawn from what the boards last said as a board switch does. */
function leaveWork(prev) {
  // Into the work view from 「すべて」: its round in flight must not write over the empty state.
  if (prev.view !== 'work' && nav.view === 'work' && prev.board === 'all' && nav.board === 'all') {
    navEpoch++;
    state = emptyState();
    return;
  }
  if (prev.view !== 'work' || nav.view === 'work' || !scopeAll()) return;
  allRound = false;
  state = Object.keys(boardStates).length ? mergeStates(readBoards(), Math.floor(Date.now() / 1000)) : emptyState();
}

function go(patch = {}, { replace = false } = {}) {
  if (boardJob && 'board' in patch && patch.board !== boardJob.slug) boardJob = null;
  const prev = { ...nav };
  Object.assign(nav, patch);
  if (nav.board !== prev.board) {
    if (!('task' in patch)) nav.task = null;
    if (!('item' in patch)) nav.item = null;
  }
  if (nav.view !== 'review') nav.item = null;
  // A session with no task is shown over the セッション tab's list: it does not follow a move off it.
  if (nav.view !== 'sessions' && nav.view !== 'work' && !('task' in patch) && isSessRef(nav.task)) { nav.task = null; nav.pane = 'detail'; }
  if (nav.view === 'review' && multiBoard) nav.board = 'all';
  const boardChanged = nav.board !== prev.board;
  if (boardChanged) switchBoard(); else leaveWork(prev);
  const url = urlOf();
  if (replace || url === location.pathname + location.search) history.replaceState(null, '', url);
  else history.pushState(null, '', url);
  applyNav(boardChanged, onlyPanelMoved(prev));
  askSessionsOfAll(prev.view, boardChanged);
}

/* Only the task panel's card or tab changed: the screen under it is as it was, and drawing it
   again would only rebuild the list under it. */
const onlyPanelMoved = prev => nav.board === prev.board && nav.view === prev.view
  && nav.item === prev.item && (nav.view === 'sessions' || nav.view === 'work' ? view === nav.view : nav.view !== 'review' && view === 'board');

/* A move to or from the セッション tab or the work view changes what the next poll asks for (the
   sessions of 「すべて」, the last lines of a board's, the work document), so it is made now. */
function askSessionsOfAll(prevView, boardChanged) {
  if (!boardChanged && ((nav.view === 'sessions') !== (prevView === 'sessions') || (nav.view === 'work') !== (prevView === 'work'))) refresh(true);
}

/* Draw the screen the address names. */
function applyNav(boardChanged, panelOnly = false) {
  const wantTask = nav.task;
  if (!panelOnly) drawScreen(boardChanged);
  // The address names the card that is open: back to one with none closes the panel, back to
  // the card that is open shows its tab, and another card is opened below.
  if (view === 'board' || view === 'sessions' || view === 'work') {
    if (!nav.task) { if (selectedTaskId) hideTaskPanelState(); }
    else if (selectedTaskId === nav.task) renderTaskPanel();
  }
  pendingTask = nav.view === 'review' ? null : wantTask;
  if (boardChanged) render();
  if (boardChanged) refresh(true);
  else applyPendingTask();
}

function drawScreen(boardChanged) {
  if (nav.view !== 'sessions') sessView.pending = false;
  navApplying = true;
  try {
    if (nav.view === 'review') {
      if (nav.item) focused = nav.item;
      setView('review');
    } else if (nav.view === 'work') {
      setView('work');
    } else if (nav.view === 'sessions') {
      if (state.boardTerminal === undefined) {
        // The first poll of this board has not said whether it has terminals.
        sessView.pending = true;
        if (view !== 'board') setView('board');
      } else if (!state.boardTerminal?.available) giveUpSessions();
      else openSessionsView();
    } else {
      prefs.tab = nav.view === 'agent' ? 'agent' : 'human';
      if (view !== 'board') setView('board'); else applyLayout();
    }
  } finally { navApplying = false; }
  if (nav.view === 'review') renderReview();
}

/* `final` is a state the board really answered with: a task it does not list is not coming. */
function applyPendingTask(final = false) {
  if (!pendingTask || (view !== 'board' && view !== 'sessions' && view !== 'work')) return;
  // A gate whose task is on this board is opened as that task, on the tab the gate is judged in.
  const landed = isGateRef(pendingTask) ? landGateRef(pendingTask) : null;
  if (landed) { pendingTask = landed.id; setNav({ task: landed.id, pane: landed.pane }); }
  // A session that has been linked to a task (or is a hub) is opened as that: the address follows.
  const id = panelRefOf(pendingTask);
  if (id !== pendingTask) { pendingTask = id; setNav({ task: id }); }
  // A parent is in the work document, not in this board's state. In the work view, a board that has
  // no such task answers for it: the panel says so (renderTaskPanel) instead of closing.
  const there = isParentRef(id) ? !!parentOfRef(id) : isHubRef(id) ? !!hubOfRef(id) : isSessRef(id) ? !!sessOfRef(id) : isGateRef(id) ? !!gateOfPanelRef(id) : (state.tasks || []).some(t => t.id === id);
  if (!there) {
    // A parent is in the work document, which may not have come yet: the pending task waits for it (refreshWork).
    if (final && !isParentRef(id)) {
      pendingTask = null;
      if (view === 'work') { selectedTaskId = id; renderTaskPanel(); }
    }
    return;
  }
  pendingTask = null;
  if (selectedTaskId !== id) showTaskPanel(id);
}

/* Run `fn` on the board `slug`: now when it is the one shown, else after switching to it and
   after its first state is in, since what `fn` opens is read from that state. */
function onBoard(slug, fn) {
  if (!slug || !multiBoard || (nav.board === slug && nav.view !== 'review')) return fn();
  go({ board: slug, view: nav.view === 'human' ? 'human' : 'agent' });
  boardJob = { slug, fn };
}
function runBoardJob() {
  if (!boardJob || nav.board !== boardJob.slug) return;
  const { fn } = boardJob;
  boardJob = null;
  fn();
}

window.addEventListener('popstate', () => {
  const next = parseUrl(location);
  boardJob = null;
  if (sameNav(next, nav)) return;
  const boardChanged = next.board !== nav.board;
  const prev = { ...nav };
  Object.assign(nav, next);
  if (boardChanged) switchBoard(); else leaveWork(prev);
  applyNav(boardChanged, onlyPanelMoved(prev));
  askSessionsOfAll(prev.view, boardChanged);
});
