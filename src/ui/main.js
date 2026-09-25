document.addEventListener('keydown', e => {
  if (e.key === 'Escape') {
    const handoverDialog = document.getElementById('handover-dialog');
    if (handoverDialog && handoverDialog.open) {
      closeHandoverDialog();
      return;
    }
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
  if (view !== 'review' || e.target.matches('textarea,input,select')) return;
  const gates = state.gates || [];
  const i = gates.findIndex(g => g.id === focused);
  if (e.key === 'j' && i < gates.length - 1) {
    focused = gates[i + 1].id;
    reviewActiveTab = TAB_OF_KIND[gates[i + 1].kind] || 'overview';
    renderReview();
  }
  if (e.key === 'k' && i > 0) {
    focused = gates[i - 1].id;
    reviewActiveTab = TAB_OF_KIND[gates[i - 1].kind] || 'overview';
    renderReview();
  }
  // A record on screen is not in the queue these keys work through, and approving one is not
  // something a record can take.
  if (i < 0) return;
  if (e.key === 'a') answer((gates[i]?.options || ['approve'])[0]);
  if (e.key === 'r') answer((gates[i]?.options || [, 'changes'])[1] || 'changes');
  if (e.key === 'c') closeGate();
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
