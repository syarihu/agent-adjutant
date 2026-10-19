/* A session terminal has the keyboard: its keys, Escape included, are the agent's. */
const inSessionTerminal = el => !!el?.closest?.('.adj-terminal');

document.addEventListener('keydown', e => {
  if (inSessionTerminal(e.target) || inSessionTerminal(document.activeElement)) return;
  if (e.key === 'Escape') {
    // An IME composition takes Escape to cancel the conversion. keyCode 229 too, as in the
    // Cmd+Enter handlers: where compositionend comes first, isComposing is already false.
    if (e.isComposing || e.keyCode === 229) return;
    // Escape is never a page shortcut: the panel and the selection close with their own buttons,
    // and a terminal that is not focused (a fresh mount, a head click) still needs Escape later.
    // An open dialog takes it, whichever it is; the task panel behind it stays.
    const dialog = document.querySelector('dialog[open]');
    if (dialog?.id === 'handover-dialog') closeHandoverDialog();
    else if (dialog?.id === 'form') dialog.close();
    // Any other dialog closes itself on Escape.
  }
  // Judging a gate has no single-key shortcuts. A bare `c` closed the gate on screen, so a
  // Cmd+C to copy from it closed it too; answering stays a click on a button.
});

/* A dialog closes itself on Escape through `cancel`, not while a terminal in it has the keyboard.
   `cancel` does not bubble, hence capture. */
document.addEventListener('cancel', e => {
  if (e.target.matches?.('dialog') && inSessionTerminal(document.activeElement)) e.preventDefault();
}, true);

function setView(v) {
  const prev = view;
  view = v;
  const boardView = document.getElementById('board-view');
  const workView = document.getElementById('work-view');
  const todoView = document.getElementById('todo-view');

  if (boardView) boardView.style.display = v === 'board' ? 'flex' : 'none';
  if (workView) workView.style.display = v === 'work' ? 'grid' : 'none';
  if (todoView) todoView.style.display = v === 'todo' ? 'flex' : 'none';
  document.body.classList.toggle('view-work', v === 'work');
  document.body.classList.toggle('view-todo', v === 'todo');
  // The TODO has no task panel of its own: the one left open on the board is closed.
  if (v === 'todo' && selectedTaskId) hideTaskPanelState();
  // The middle terminal is not kept behind another view, and the row that was open is left.
  if (prev === 'work' && v !== 'work') { workLeave(); disposeTermSlot(workTerm); disposeTermSlot(aideTerm); }

  // Navigation rail active states
  // The sidebar's switch follows the view: 「いまの仕事」 and TODO are tabs of their own, every other view is ボード's.
  for (const [id, on] of [['side-tab-boards', v !== 'work' && v !== 'todo'], ['nav-work', v === 'work'], ['nav-todo', v === 'todo']])
    document.getElementById(id)?.setAttribute('aria-selected', String(on));
  applyRailMode();
  applyLayout();

  // The address follows the screen when something other than `go` moved it.
  if (!navApplying) {
    const want = v === 'board' ? (prefs.tab === 'human' ? 'human' : 'agent') : v;
    if (nav.view !== want) {
      nav.view = want;
      if (want !== 'work' && (isSessRef(nav.task) || isPrRef(nav.task))) { nav.task = null; nav.pane = 'detail'; }
      history.replaceState(null, '', urlOf());
    }
  }

  // Top App Bar title updates. The board's own are drawn by renderTitle.
  const pageTitle = document.getElementById('page-title');
  const pageSub = document.getElementById('page-subtitle');
  if (pageTitle && pageSub) {
    if (v === 'board') {
      renderTitle();
    } else if (v === 'work') {
      pageTitle.textContent = 'いまの仕事';
      pageSub.textContent = '全リポジトリの動いている仕事。左で選び、中央で話し、右で判定する';
    } else if (v === 'todo') {
      pageTitle.textContent = 'TODO';
      pageSub.textContent = 'あなたの番。すぐ答えられるものから';
    }
  }

  renderTitle();
  // Filter chips are visible on board
  const filterChips = document.querySelector('.filter-chip-group');
  if (filterChips) filterChips.style.display = (v === 'board') ? 'flex' : 'none';

  if (v === 'work') {
    renderWorkView();
    renderTaskPanel();
  } else if (v === 'todo') {
    todoFollow(true);
  } else {
    if (/^#(task|gate|sessions?)(\/|$)/.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
    render();
  }
}

function toggleBottomSheet() {
  const sheet = document.getElementById('m3-bottom-sheet');
  if (!sheet) return;
  sheet.classList.toggle('open');
  const isOpen = sheet.classList.contains('open');
  const icon = document.getElementById('sheet-toggle-icon');
  if (icon) icon.textContent = isOpen ? '▼ 閉じる' : '▲ 開く';
  const handleBar = sheet.querySelector('.sheet-handle-bar');
  if (handleBar) handleBar.setAttribute('aria-expanded', String(isOpen));
}

async function refreshAll(base = BASE) {
  const line = 'adj refresh';
  try {
    await boardApi(base, '/api/refresh', { method: 'POST' });
    note(line, false, '外部同期を完了しました');
    await refresh(true);
  } catch (e) { note(`${line} → ${e.message}`, true); }
}


function toggleTheme() {
  const cur = document.documentElement.dataset.theme ||
    (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  document.documentElement.dataset.theme = cur === 'dark' ? 'light' : 'dark';
}

// Each entry is an arrow so a later optional first parameter never receives the element. One
// entry per line: the_page_wires_no_inline_handlers reads the keys line by line.
const ACTIONS = {
  'new-task': () => openForm(),
  work: () => { if (view !== 'work') go({ board: 'all', view: 'work', task: null, pane: 'detail' }); },
  todo: () => todoOpen(),
  boards: () => { if (view === 'work' || view === 'todo') go({ board: 'all', view: prefs.tab === 'human' ? 'human' : 'agent', task: null, pane: 'detail' }); },
  'rail-toggle': () => { prefs.railHidden = !prefs.railHidden; if (prefs.railHidden) closeOrgMenu(); applyLayout(); },
  notify: () => toggleNotify(),
  'notify-permission': () => requestNotifyFromDialog(),
  theme: () => toggleTheme(),
  'own-hub': () => openOwnHub(),
  resync: () => refreshAll(),
  layout: el => setBoardLayout(el.dataset.layout),
  arrange: el => setBoardArrange(el.dataset.arrange),
  sheet: () => toggleBottomSheet(),
  'handover-close': () => closeHandoverDialog(),
  'nudge-hub': () => nudgeHub(),
  'wake-hub': () => wakeHub(),
  'hub-title-read': () => readHubTitle(pageHub(), BASE),
  'hub-strip-open': () => { const h = pageHub(); if (h) openTaskPanel(HUB_REF + h.id, 'detail'); },
};
document.addEventListener('click', e => {
  const el = e.target.closest('[data-action]');
  if (el) ACTIONS[el.dataset.action](el);
});

/* Whether this server serves several boards: the resident answers the list of them, a board
   served on its own does not know the route. */
async function probeBoards() {
  if (multiBoard) return fetchBoards();
  try {
    boards = await boardApi('', '/api/boards');
    multiBoard = true;
  } catch {
    multiBoard = false;
  }
}

async function boot() {
  await probeBoards();
  document.body.classList.toggle('single-board', !multiBoard);
  // And #task/<id>/<tab>, from before the panel had the tabs: the task's panel on that tab.
  const deepTask = location.hash.match(/^#task\/([^/]+)(?:\/(\w+))?$/);
  // A malformed escape in a hand-edited link would throw here, before the polling below starts,
  // and leave a page that never loads: such a link opens the board instead.
  let deepTaskId = null;
  try { deepTaskId = deepTask && decodeURIComponent(deepTask[1]); } catch { deepTaskId = null; }
  // The old addresses of the セッション tab and of 要対応 (#gate/<id>, #session/<id>, /review, view=sessions), which
  // parseUrl has already sent to where they land (nav-legacy.js); a gate they name is opened once the work
  // document says where it is.
  const legacy = legacyNav(location, multiBoard);
  Object.assign(nav, parseUrl(location));
  BASE = multiBoard && nav.board && nav.board !== 'all' ? `/b/${nav.board}` : '';
  pendingGate = legacy?.pendingGate || null;
  // 「すべて」 has no task panel in its address (parseUrl drops `task` there), so the link opens that
  // board's list as it is.
  const taskLinked = !!deepTaskId && nav.board !== 'all';
  if (taskLinked) {
    nav.view = 'agent';
    nav.task = deepTaskId;
    nav.pane = (tabs => Object.hasOwn(tabs, deepTask[2]) ? tabs[deepTask[2]] : 'detail')({ overview: 'detail', review: 'review', check: 'check', history: 'history' });
  }
  // The address in its own spelling, so a go() to the same screen does not push a twin.
  history.replaceState(null, '', urlOf() + (legacy || taskLinked ? '' : location.hash));
  // The page's sections are shown or hidden by the first `setView`; the address has the say on
  // which view that is, so it is not allowed to rewrite it.
  navApplying = true;
  setView('board');
  navApplying = false;
  // Draws the screen the address names and starts its first poll.
  applyNav(true);
  updateNotifyButton();
  // The server holds no clock, so the page carries one: it asks, nothing pushes.
  setInterval(refresh, 2000);
  // The sidebar's count of what is new is the work document's, so the views that do not poll it ask now and then.
  Promise.all([refreshWork(), refreshOthers()]).then(todoPoll);
  setInterval(() => { refreshBoards(); if (view !== 'work') Promise.all([refreshWork(), refreshOthers()]).then(todoPoll); }, 10000);
}
boot();
