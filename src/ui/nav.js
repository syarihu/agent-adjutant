/* ── Where the page is: one board, every board, or the work under way ────────────────────────
   Navigation state lives in the address, so back/forward and a pasted link land on the same
   screen without a reload; what is only a preference (layout, folded repositories) does not.
     /b/<slug>/?task=<id>&pane=term                        one board; pane is detail (the default),
                                                           term, review, check or history. On the resident server
                                                           term is the terminal beside the board (boardSideOn)
     /b/<slug>/?task=hub:<id>&pane=term                    one board, a hub in the panel
     /b/<slug>/?task=session:<id>                          one board, a session with no task in the panel
     /b/<slug>/?task=gate:<slug>/<id>                      one board, a gate in the panel (`gate:<id>` on a board
                                                           served alone)
     /                                                     いまの仕事, every board (the view DEFAULT_MULTI_VIEW names)
     /?view=agent                                          すべて, every board
     /?view=todo                                           TODO, every board: what waits on you, and the question selected
     /?view=work                                           いまの仕事, every board; nothing selected, so the
                                                           middle is the empty state
     /b/<slug>/?view=work&task=<ref>&pane=detail           いまの仕事 with a row selected: the board of
                                                           the row's task, hub or parent (<ref> may also
                                                           be parent:<key>)
     /?view=review                                         レビュー, every board; nothing selected, so the
                                                           middle is the empty state
     /?view=review&task=pr:<owner/repo#N>&pane=detail      レビュー with a PR waiting on your review in the
                                                           middle (pane: detail = 下読み, which holds the diff, or drafts; an older pane=diff lands on detail), its reader on the right
                                                           (/?view=work&task=pr:... from older links lands on the same)
   /b/<slug>/?view=work&task=<id>&pane=ai                 an older link to a task's AI 下読み (pane: ai, or ai-drafts): the task opens
                                                           in いまの仕事 and its AI 下読み dialog opens over it
   A board served on its own has no list of boards, so it is `board: null` at `/`. Every
   address carries `?token=`: the server refuses a GET without it. The addresses of the セッション
   tab and of 要対応, which are gone, are sent on by nav-legacy.js. */
/* The task panel's tabs, as `pane=` names them; anything else is 詳細 (タスクサマリ). */
const PANES = ['detail', 'term', 'review', 'check', 'history'];
const nav = { board: null, view: 'agent', list: 'task', task: null, pane: 'detail' };
let boards = [];                 // /api/boards: the sidebar's rows
let multiBoard = /^\/b\//.test(location.pathname);   // the resident server: more than one board
let pendingOwnRead = null;       // '<board>/<task id>' of an older AI 下読み link: its dialog opens once the task is drawn (task-read-view.js)
let navEpoch = 0;                // bumped on a board switch, so a late answer for the old one is dropped
let navApplying = false;
let pendingTask = null;          // opened once the board's first state is in
let pendingGate = null;          // `<board>/<gate>` of an old link, opened once the work document is in (my-work.js)
let boardJob = null;             // { slug, fn }: run once that board has loaded
const boardStates = {};          // 「すべて」: each board's last state, by slug
const baseOf = x => x?._base || BASE;
/* 「すべて」 is the merged board. The work view has no board of its own while nothing is selected, and
   reads its own document instead (my-work.js): it is not 「すべて」, whatever `nav.board` says. */
const scopeAll = () => multiBoard && nav.board === 'all' && nav.view !== 'work';
/* No board is chosen: 「すべて」, or the work view with nothing selected. What would be sent to a board is asked for one. */
const noBoard = () => multiBoard && (!nav.board || nav.board === 'all');
/* The view `/` opens on a server that serves several boards. A constant so that one change moves it. */
const DEFAULT_MULTI_VIEW = 'work';

function parseUrl(loc = location) {
  const q = new URLSearchParams(loc.search);
  const task = q.get('task');
  const p = q.get('pane');
  // A PR has its own tabs; any other task has the board's.
  const out = { board: null, view: 'agent', list: 'task', task, pane: PANES.includes(p) || (isPrRef(task) && PR_PANES.includes(p)) ? p : 'detail' };
  const m = /^\/b\/([^/]+)/.exec(loc.pathname);
  out.board = m ? m[1] : multiBoard ? 'all' : null;
  // `/` on a server with several boards opens the view its constant names; a board's own address, the one before.
  if (multiBoard && out.board === 'all') out.view = DEFAULT_MULTI_VIEW;
  const v = q.get('view');
  // `/` on a resident server is the work view, so 「すべて」 has to be asked for by name. The human board is gone: its old address opens the board.
  if (v === 'human' || v === 'agent') out.view = 'agent';
  // The work view reads every board of a resident server: a board served alone has none.
  if (v === 'work' && multiBoard) out.view = 'work';
  // 「レビュー」 is the work view on a resident server with list: 'review' and board: 'all'.
  if (v === 'review' && multiBoard) { out.view = 'work'; out.list = 'review'; out.board = 'all'; }
  // The TODO reads every board too, and is on none of them.
  if (v === 'todo' && multiBoard) { out.view = 'todo'; out.board = 'all'; }
  // The addresses of the セッション tab and of 要対応, which are gone, land where nav-legacy.js says.
  const legacy = legacyNav(loc, multiBoard);
  if (legacy) Object.assign(out, legacy.nav);
  // With view=review, a task that is not a PR ref is dropped.
  if (v === 'review' && !isPrRef(out.task)) out.task = null;
  // The task panel opens on a board of its own; 「すべて」 switches to the card's board first.
  // A PR waiting on your review belongs to no board: its address stays on 「すべて」, in the work view.
  if (out.board === 'all' && !(out.view === 'work' && isPrRef(out.task))) out.task = null;
  // And it is kept nowhere else: another view, or a board's own address (a board served alone never reads the records), has no panel for it.
  if (isPrRef(out.task) && (out.view !== 'work' || out.board !== 'all')) out.task = null;
  // An older link to a task's AI 下読み opens the task, and the dialog of its read-through once the task is drawn (pendingOwnRead).
  if (/^ai(-drafts)?$/.test(p) && out.view === 'work' && isTaskRef(out.task)) pendingOwnRead = `${out.board}/${out.task}`;
  // A PR address (with view=work as older links have it, or view=review) is list: 'review'. Everything else is list: 'task'.
  if (out.view !== 'work') out.list = 'task';
  else if (isPrRef(out.task)) out.list = 'review';
  return out;
}

function urlOf(n = nav) {
  const path = n.board && n.board !== 'all' ? `/b/${n.board}/` : '/';
  let url = path + '?token=' + encodeURIComponent(TOKEN);
  if (n.view === 'work') url += n.list === 'review' ? '&view=review' : '&view=work';
  else if (n.view === 'todo') url += `&view=${n.view}`;
  // 「すべて」 is what `/` no longer opens.
  else if (n.view === 'agent' && multiBoard && path === '/') url += '&view=agent';
  if (n.task) url += `&task=${encodeURIComponent(n.task)}`;
  if (n.task && n.pane !== 'detail') url += `&pane=${n.pane}`;
  return url;
}

/* A change of address that is not a move: the card being looked at, the gate being read. */
function setNav(patch) {
  Object.assign(nav, patch);
  history.replaceState(null, '', urlOf());
}

const sameNav = (a, b) => a.board === b.board && a.view === b.view && a.list === b.list && a.task === b.task && a.pane === b.pane;

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
  // 「すべて」 shows what it last read while the new round is on its way.
  allRound = false;
  state = scopeAll() && Object.keys(boardStates).length ? mergeStates(readBoards(), Math.floor(Date.now() / 1000)) : emptyState();
}

/* From 「レビュー」 to 「いまの仕事」 with nothing selected: the view did not change, so `setView` does not bring back the session
   selected last; it is asked for here, as arriving from another view asks for it. */
function backToWorkList(prev) {
  if (prev.view === 'work' && prev.list === 'review' && nav.view === 'work' && nav.list === 'task' && !nav.task) work.restore = true;
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
  // The person moves on: an old link's gate is no longer wanted.
  pendingGate = null;
  if (boardJob && 'board' in patch && patch.board !== boardJob.slug) boardJob = null;
  const prev = { ...nav };
  Object.assign(nav, patch);
  if (nav.board !== prev.board && !('task' in patch)) nav.task = null;
  // A session with no task is shown over 「いまの仕事」: it does not follow a move off it.
  // A PR waiting on your review is shown over 「いまの仕事」 only, as a session with no task is.
  if (nav.view !== 'work' && !('task' in patch) && (isSessRef(nav.task) || isPrRef(nav.task))) { nav.task = null; nav.pane = 'detail'; }
  if (nav.view !== 'work') nav.list = 'task';
  else if (isPrRef(nav.task)) nav.list = 'review';
  else if (nav.task) nav.list = 'task';
  else if ('list' in patch) nav.list = patch.list;
  // The person moves on: an old link's dialog is no longer wanted.
  if (pendingOwnRead && pendingOwnRead !== `${nav.board}/${nav.task}`) pendingOwnRead = null;
  const boardChanged = nav.board !== prev.board;
  backToWorkList(prev);
  if (boardChanged) switchBoard(); else leaveWork(prev);
  const url = urlOf();
  if (replace || url === location.pathname + location.search) history.replaceState(null, '', url);
  else history.pushState(null, '', url);
  applyNav(boardChanged, onlyPanelMoved(prev));
  askWorkOnMove(prev.view, boardChanged);
}

/* Only the task panel's card or tab changed: the screen under it is as it was, and drawing it
   again would only rebuild the list under it. */
const onlyPanelMoved = prev => nav.board === prev.board && nav.view === prev.view && nav.list === prev.list
  && (nav.view === 'work' ? view === 'work' : view === 'board');

/* A move to or from the work view changes what the next poll asks for (the work document, the last
   lines of a board's), so it is made now. */
function askWorkOnMove(prevView, boardChanged) {
  if (!boardChanged && ((nav.view === 'work') !== (prevView === 'work') || (nav.view === 'todo') !== (prevView === 'todo'))) refresh(true);
}

/* Draw the screen the address names. */
function applyNav(boardChanged, panelOnly = false) {
  const wantTask = nav.task;
  if (!panelOnly) drawScreen(boardChanged);
  // The address names the card that is open: back to one with none closes the panel, back to
  // the card that is open shows its tab, and another card is opened below.
  if (view === 'board' || view === 'work') {
    if (!nav.task) { if (selectedTaskId || boardSideShown) hideTaskPanelState(); }
    else if (selectedTaskId === nav.task) renderTaskPanel();
  }
  pendingTask = wantTask;
  if (boardChanged) render();
  if (boardChanged) refresh(true);
  else applyPendingTask();
}

function drawScreen(boardChanged) {
  navApplying = true;
  try {
    if (nav.view === 'work') {
      setView('work');
    } else if (nav.view === 'todo') {
      setView('todo');
    } else {
      if (view !== 'board') setView('board'); else applyLayout();
    }
  } finally { navApplying = false; }
}

/* `final` is a state the board really answered with: a task it does not list is not coming. */
function applyPendingTask(final = false) {
  if (!pendingTask || (view !== 'board' && view !== 'work')) return;
  // A gate whose task is on this board is opened as that task, on the tab the gate is judged in.
  const landed = isGateRef(pendingTask) ? landGateRef(pendingTask) : null;
  if (landed) { pendingTask = landed.id; setNav({ task: landed.id, pane: landed.pane }); }
  // A session that has been linked to a task (or is a hub) is opened as that: the address follows.
  const id = panelRefOf(pendingTask);
  if (id !== pendingTask) { pendingTask = id; setNav({ task: id }); }
  // A parent is in the work document, not in this board's state. In the work view, a board that has
  // no such task answers for it: the panel says so (renderTaskPanel) instead of closing.
  const there = isPrRef(id) ? prReady() : isParentRef(id) ? !!parentOfRef(id) : isHubRef(id) ? !!hubOfRef(id) : isSessRef(id) ? !!sessOfRef(id) : isGateRef(id) ? !!gateOfPanelRef(id) : (state.tasks || []).some(t => t.id === id);
  if (!there) {
    // A parent is in the work document, which may not have come yet: the pending task waits for it (refreshWork).
    if (final && !isParentRef(id) && !isPrRef(id)) {
      pendingTask = null;
      if (view === 'work' || boardSideOn()) { selectedTaskId = id; renderTaskPanel(); }
    }
    return;
  }
  pendingTask = null;
  // A gate that landed on its task has selected it already, and still has to be drawn.
  if (landed || selectedTaskId !== id) showTaskPanel(id);
}

/* Run `fn` on the board `slug`: now when it is the one shown, else after switching to it and
   after its first state is in, since what `fn` opens is read from that state. */
function onBoard(slug, fn) {
  if (!slug || !multiBoard || nav.board === slug) return fn();
  go({ board: slug, view: 'agent' });
  boardJob = { slug, fn };
}
function runBoardJob() {
  if (!boardJob || nav.board !== boardJob.slug) return;
  const { fn } = boardJob;
  boardJob = null;
  fn();
}

window.addEventListener('popstate', () => {
  pendingGate = null;
  // An older link's dialog is armed again only if the address now shown carries it.
  pendingOwnRead = null;
  const next = parseUrl(location);
  boardJob = null;
  if (sameNav(next, nav)) return;
  const boardChanged = next.board !== nav.board;
  const prev = { ...nav };
  Object.assign(nav, next);
  backToWorkList(prev);
  if (boardChanged) switchBoard(); else leaveWork(prev);
  applyNav(boardChanged, onlyPanelMoved(prev));
  askWorkOnMove(prev.view, boardChanged);
});
