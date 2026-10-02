document.addEventListener('keydown', e => {
  // Keys typed in a session terminal belong to the agent, Escape included.
  if (e.target.closest?.('.adj-terminal')) return;
  if (e.key === 'Escape') {
    // An IME composition takes Escape to cancel the conversion. keyCode 229 too, as in the
    // Cmd+Enter handlers: where compositionend comes first, isComposing is already false.
    if (e.isComposing || e.keyCode === 229) return;
    // An open dialog takes Escape, whichever it is; the task panel or task view behind it stays.
    const dialog = document.querySelector('dialog[open]');
    if (dialog) {
      if (dialog.id === 'handover-dialog') closeHandoverDialog();
      else if (dialog.id === 'form') dialog.close();
      // Any other dialog closes itself on Escape.
      return;
    }
    // Escape in a text field must not close what it is in and drop what was typed.
    if (e.target.matches('textarea,input,select')) return;
    if (view === 'task') {
      backToBoard();
      return;
    }
    if ((view === 'board' || view === 'sessions') && selectedTaskId) {
      // A popped-out panel goes back to its side first; the next Escape closes it.
      if (panelPop) { panelPop = false; renderTaskPanel(); } else closeTaskPanel();
      return;
    }
  }
  // The review view has no single-key shortcuts. A bare `c` closed the gate on screen, so a
  // Cmd+C to copy from it closed it too; answering stays a click on a button.
});

function setView(v) {
  // The review queue reads every board: on a server with several, it is a page of its own.
  if (v === 'review' && multiBoard && !navApplying && !scopeAll()) return go({ view: 'review' });
  const prev = view;
  view = v;
  const boardView = document.getElementById('board-view');
  const reviewView = document.getElementById('review');
  const taskView = document.getElementById('task-view');
  const sessionsView = document.getElementById('sessions-view');

  if (boardView) boardView.style.display = v === 'board' ? 'flex' : 'none';
  if (reviewView) {
    reviewView.classList.toggle('on', v === 'review');
    reviewView.style.display = v === 'review' ? 'grid' : 'none';
  }
  if (taskView) {
    taskView.classList.toggle('on', v === 'task');
    taskView.style.display = v === 'task' ? 'grid' : 'none';
  }

  if (sessionsView) sessionsView.style.display = v === 'sessions' ? 'grid' : 'none';
  if (prev === 'sessions' && v !== 'sessions') leaveSessionsView();
  // The review view's terminal is not kept behind another view.
  if (prev === 'review' && v !== 'review') disposeTermSlot(reviewTerm);

  // Navigation rail active states
  const navReview = document.getElementById('nav-review');
  if (navReview) {
    if (v === 'review') navReview.setAttribute('aria-current', 'page');
    else navReview.removeAttribute('aria-current');
  }
  applyLayout();

  // The address follows the screen when something other than `go` moved it.
  if (!navApplying) {
    const want = v === 'board' ? (prefs.tab === 'agent' ? 'agent' : 'human') : v === 'review' ? 'review' : v === 'sessions' ? 'sessions' : null;
    if (want && nav.view !== want) {
      nav.view = want;
      if (want !== 'sessions' && isSessRef(nav.task)) { nav.task = null; nav.pane = 'detail'; }
      history.replaceState(null, '', urlOf());
    }
  }

  // Top App Bar title updates. The board's own are drawn by renderTitle.
  const pageTitle = document.getElementById('page-title');
  const pageSub = document.getElementById('page-subtitle');
  if (pageTitle && pageSub) {
    if (v === 'board' || v === 'sessions') {
      renderTitle();
    } else if (v === 'review') {
      pageTitle.textContent = '要対応レビュー';
      pageSub.textContent = '全ボードの判断待ち。左で選んで、右で答える';
    } else if (v === 'task') {
      pageTitle.textContent = 'タスク詳細';
      pageSub.textContent = '個別タスクの全工程記録と実行タイムライン';
    }
  }

  renderTitle();
  // Filter chips are visible on board
  const filterChips = document.querySelector('.filter-chip-group');
  if (filterChips) filterChips.style.display = (v === 'board') ? 'flex' : 'none';

  if (v === 'task') {
    taskViewShown = null;
    // Out of sight, not closed: the board comes back with the panel on the same card.
    renderTaskPanel();
    renderTaskView();
  } else if (v === 'sessions') {
    renderSessionsTab();
    renderSessionsView();
    renderTaskPanel();
  } else if (v !== 'board') {
    dismissTaskPanel();
  } else {
    if (/^#(task|gate|sessions?)(\/|$)/.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
    if (!(state.gates || []).some(g => gateRef(g) === focused)) focused = null;
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

note('adj serve', false, 'このボードの操作は既存の adj コマンドに対応しています');

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
  // Opening #gate/<id> lands straight on that card in the review queue.
  const deep = location.hash.match(/^#gate\/(.+)$/);
  // And #task/<id>/<tab> on one task's view.
  const deepTask = location.hash.match(/^#task\/([^/]+)(?:\/(\w+))?$/);
  // A malformed escape in a hand-edited link would throw here, before the polling below starts,
  // and leave a page that never loads: such a link opens the board instead.
  let deepTaskId = null;
  try { deepTaskId = deepTask && decodeURIComponent(deepTask[1]); } catch { deepTaskId = null; }
  // #session/<id> (or #sessions), from before the address named it: the セッション tab. Whether a
  // terminal exists is known only once the first poll is in, so the board is shown meanwhile.
  const deepSession = location.hash.match(/^#sessions?(?:\/(.+))?$/);
  let deepSessionId = null;
  try { deepSessionId = deepSession && deepSession[1] ? decodeURIComponent(deepSession[1]) : null; } catch { deepSessionId = null; }
  Object.assign(nav, parseUrl(location));
  BASE = multiBoard && nav.board && nav.board !== 'all' ? `/b/${nav.board}` : '';
  if (deepSession) {
    nav.view = 'sessions';
    if (nav.board !== 'all' && deepSessionId) { nav.task = SESS_REF + deepSessionId; nav.pane = 'term'; }
  }
  // A link made before the address named the review queue: carry its gate over.
  if (deep) { nav.view = 'review'; nav.item = (m => m ? `${m[1]}/${deep[1]}` : deep[1])(/^\/b\/([^/]+)/.exec(location.pathname)); if (multiBoard) nav.board = 'all'; }
  // The address in its own spelling, so a go() to the same screen does not push a twin.
  history.replaceState(null, '', urlOf() + (deep || deepSession ? '' : location.hash));
  // The page's sections are shown or hidden by the first `setView`; the address has the say on
  // which view that is, so it is not allowed to rewrite it.
  navApplying = true;
  setView('board');
  navApplying = false;
  if (deepTaskId) {
    // The task's own view on the board the address names; the hash stays as it is.
    nav.view = 'human';
    const epoch = navEpoch;
    refresh(true).then(() => {
      // The person may have gone to another board while the first state was on its way.
      if (epoch !== navEpoch) return;
      openTask(deepTaskId, TASK_TABS.some(([id]) => id === deepTask[2]) ? deepTask[2] : 'overview');
    });
  } else {
    // Draws the screen the address names and starts its first poll.
    applyNav(true);
  }
  updateNotifyButton();
  // The server holds no clock, so the page carries one: it asks, nothing pushes.
  setInterval(refresh, 2000);
  setInterval(refreshBoards, 10000);
}
boot();
