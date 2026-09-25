document.addEventListener('keydown', e => {
  if (e.key === 'Escape') {
    const handoverDialog = document.getElementById('handover-dialog');
    if (handoverDialog && handoverDialog.open) {
      closeHandoverDialog();
      return;
    }
    // The dialog closes itself on Escape; the side sheet it was opened from stays.
    if (document.getElementById('close-dialog').open) return;
    const formDialog = document.getElementById('form');
    if (formDialog && formDialog.open) {
      formDialog.close();
      return;
    }
    // Escape in a comment box must not leave the task and drop what was typed.
    if (view === 'task' && !e.target.matches('textarea,input,select')) {
      backToBoard();
      return;
    }
    if (view === 'board' && selectedTaskId) {
      closeDrawer();
      return;
    }
  }
  // The review view has no single-key shortcuts. A bare `c` closed the gate on screen, so a
  // Cmd+C to copy from it closed it too; answering stays a click on a button.
});

function setView(v) {
  view = v;
  const boardView = document.getElementById('board-view');
  const reviewView = document.getElementById('review');
  const taskView = document.getElementById('task-view');

  if (boardView) boardView.style.display = v === 'board' ? 'flex' : 'none';
  if (reviewView) {
    reviewView.classList.toggle('on', v === 'review');
    reviewView.style.display = v === 'review' ? 'grid' : 'none';
  }
  if (taskView) {
    taskView.classList.toggle('on', v === 'task');
    taskView.style.display = v === 'task' ? 'grid' : 'none';
  }

  // Navigation rail active states
  const navBoard = document.getElementById('nav-board');
  const navReview = document.getElementById('nav-review');
  for (const [nav, on] of [[navBoard, v === 'board'], [navReview, v === 'review']]) {
    if (!nav) continue;
    if (on) nav.setAttribute('aria-current', 'page'); else nav.removeAttribute('aria-current');
  }

  // Top App Bar title updates
  const pageTitle = document.getElementById('page-title');
  const pageSub = document.getElementById('page-subtitle');
  if (pageTitle && pageSub) {
    if (v === 'board') {
      pageTitle.textContent = 'タスクボード';
      pageSub.textContent = 'バックグラウンド worker との協調作業カンバン';
    } else if (v === 'review') {
      pageTitle.textContent = '要対応レビュー';
      pageSub.textContent = '人間の判断・承認を待っている Gate 一覧';
    } else if (v === 'task') {
      pageTitle.textContent = 'タスク詳細';
      pageSub.textContent = '個別タスクの全工程記録と実行タイムライン';
    }
  }

  // Filter chips are visible on board
  const filterChips = document.querySelector('.filter-chip-group');
  if (filterChips) filterChips.style.display = (v === 'board') ? 'flex' : 'none';

  if (v === 'task') {
    taskViewShown = null;
    renderDrawer();
    renderTaskView();
  } else if (v !== 'board') {
    closeDrawer();
  } else {
    if (/^#(task|gate)\//.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
    if (!(state.gates || []).some(g => g.id === focused)) focused = null;
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

async function refreshAll() {
  const line = 'adj refresh';
  try {
    await api('/api/refresh', { method: 'POST' });
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
// Opening #gate/<id> lands straight on that card in the review view.
const deep = location.hash.match(/^#gate\/(.+)$/);
// And #task/<id>/<tab> on one task's view.
const deepTask = location.hash.match(/^#task\/([^/]+)(?:\/(\w+))?$/);
// A malformed escape in a hand-edited link would throw here, before the polling below starts,
// and leave a page that never loads: such a link opens the board instead.
let deepTaskId = null;
try { deepTaskId = deepTask && decodeURIComponent(deepTask[1]); } catch { deepTaskId = null; }
if (deep) { focused = deep[1]; setView('review'); }
else if (deepTaskId) openTask(deepTaskId, TASK_TABS.some(([id]) => id === deepTask[2]) ? deepTask[2] : 'overview');
else { setView('board'); }
refresh();
updateNotifyButton();
// The server holds no clock, so the page carries one: it asks, nothing pushes.
setInterval(refresh, 2000);
