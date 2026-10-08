// ── the actions, each one logging the adj command it maps to ──────────

async function move(id, to, before) {
  // Cards of several boards share one queue order only on their own board.
  if (scopeAll()) return;
  const task = (state.tasks || []).find(t => t.id === id);
  if (!task || !canDrop(columnOf(task), to)) return;
  if (columnOf(task) === 'backlog' && to === 'queued') return openHandoverDialog(id, before);
  if (to === 'backlog') {
    await update(id, { status:'backlog' },
      `adj task update --id ${id} --status backlog`,
      'hub は着手の直前に status を読むため、戻したものはスキップされます');
    return;
  }
  // Reordering inside 待ち: the dropped card takes the position of the one it landed on.
  const order = await queueOrder(id, before);
  if (order == null) return;
  await update(id, { order }, `adj task update --id ${id} --order ${order}`,
    'hub がキューの先頭から取得する順序です');
}

/* The order that puts a task just before `before` in 待ち, or at its end without one.
   Orders are not positions: a new task takes the highest order of every task plus one, so the
   queue can read 12, 15, 20. The task takes the gap below the one it lands on. When there is
   no gap, that one and the ones after it move down first, so no two queued tasks share an
   order. They move from the last one up, each into an order nobody holds, so a request that
   fails part way leaves the queue in the same sequence with no order shared. Null then. */
async function queueOrder(id, before) {
  if (scopeAll()) return null;
  const queue = (state.tasks || []).filter(t => t.status === 'queued' && t.id !== id)
    .sort((a, b) => (a.order || 0) - (b.order || 0));
  const at = before ? queue.findIndex(t => t.id === before) : -1;
  if (at < 0) return (queue.length ? (queue[queue.length - 1].order || 0) : 0) + 1;
  const floor = at > 0 ? (queue[at - 1].order || 0) + 1 : 0;
  const target = queue[at].order || 0;
  if (target > floor) return target - 1;
  const shifts = [];
  let next = floor + 1;
  for (const t of queue.slice(at)) {
    if ((t.order || 0) >= next) break;
    shifts.push([t.id, next++]);
  }
  for (const [tid, order] of shifts.reverse()) {
    const line = `adj task update --id ${tid} --order ${order}`;
    try {
      await api(`/api/tasks/${encodeURIComponent(tid)}`, {
        method:'POST', body: JSON.stringify({ order }),
      });
      note(line, false, '間に入れる場所を空けるため後ろへずらしました');
    } catch (e) {
      note(`${line} → ${e.message}`, true);
      return null;
    }
  }
  return floor;
}

let handoverTargetTaskId = null;
let handoverTargetBefore = null;

/* Opened by an IDE button while no editor is configured: what to write, and where. */
function openIdeDialog() {
  document.getElementById('ide-config-path').textContent = state.configPath || '';
  document.getElementById('ide-dialog').showModal();
}

/* Asked before a worker's tab is closed, in the page rather than with confirm(), which reads
   like any other browser prompt. Only the button that says so closes it: cancel, Escape and the
   header's close button all leave the worker running. */
let closeTarget = null;
function openCloseDialog(worktree) {
  closeTarget = worktree;
  document.getElementById('close-worktree-name').textContent = baseName(worktree);
  document.getElementById('close-worktree-path').textContent = worktree;
  const dialog = document.getElementById('close-dialog');
  dialog.returnValue = '';
  dialog.showModal();
}
document.getElementById('close-dialog').addEventListener('close', e => {
  const worktree = closeTarget;
  closeTarget = null;
  // A worker that stopped while the dialog was open has no tab left to close.
  const running = (state.workers || []).some(w => w.worktree === worktree && w.present);
  if (e.target.returnValue === 'close' && worktree && running) worktreeAct('close', worktree, true);
});

/* How the Sessions view names a hub. */
function hubLabel(h) {
  if (!h.parent) return 'リポジトリの hub';
  return h.key ? `親タスク ${h.key} の hub` : '親タスクの hub（キー不明）';
}
/* The name a hub's row has room for; the whole of `hubLabel` is its title. */
function hubShortName(h) {
  if (!h.parent) return 'リポジトリ';
  return h.key || '親タスク（キー不明）';
}
/* What a hub's row is titled: the repository's name for its own hub, the parent task's title for
   a parent-task hub (null until it is known, when the row keeps its key). */
function hubTitle(h, data = state) {
  if (!h.parent) return repoName(data);
  return h.title || null;
}
function repoName(data = state) {
  return (data.repo || '').split('/').pop() || null;
}
/* The hub this page is served for; the list always includes the board's own. */
function pageHub() {
  return (state.hubs || []).find(h => h.name === state.hubName) || null;
}
/* The tab's title. The board's name comes first so a narrow tab still shows it, and a parent-task
   hub's title is the one the session tree shows (`hubTitle`), so the two never disagree. */
function boardTitle() {
  if (view === 'work') return 'いまの仕事 — adj';
  if (scopeAll()) return nav.view === 'review' ? '要対応 — adj' : 'すべて — adj';
  const entry = selectedBoard();
  if (entry) {
    const repo = entry.nwo.split('/').pop();
    return entry.hub ? `${boardName(entry)} — ${repo}` : `${repo} — adj`;
  }
  const repo = repoName();
  if (!repo) return 'adj';
  const h = pageHub();
  if (!h || !h.parent) return `${repo} — adj`;
  const label = h.key ? [h.key, hubTitle(h)].filter(Boolean).join(' ') : hubShortName(h);
  return `${label} — ${repo}`;
}

/* ── the boards in the sidebar ── */
const selectedBoard = () => multiBoard && nav.board && nav.board !== 'all' ? boards.find(b => b.slug === nav.board) || null : null;
const repoNameOf = b => (b.nwo || '').split('/').pop() || b.nwo || '';
/* A repository's board is named for the repository, a parent-task hub's for its key and, once
   it is known, the parent task's title. */
const boardName = b => b.hub ? [b.hub, b.title].filter(Boolean).join(' ') : repoNameOf(b);

function initialsOf(b) {
  if (b.hub) return (/(\d+)\s*$/.exec(b.hub) || [])[1] || b.hub.slice(0, 3);
  const name = repoNameOf(b);
  const parts = name.split(/[-_.]/).filter(Boolean);
  return (parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2)).toUpperCase();
}

const rowStartedAt = {};
const rowStartingNow = b => rowStartedAt[b.slug] != null && Date.now() - rowStartedAt[b.slug] < HUB_STARTING_MS;

/* The hub's state in the words a row has room for. */
function boardState(b) {
  if (b.hubPresent) return { tone: 'good', text: b.hub ? '親タスク hub' : 'リポジトリ hub', stopped: false };
  const since = sinceLabel(b.hubLastAlive);
  return {
    tone: b.hubStale ? 'bad' : 'off',
    text: rowStartingNow(b) ? '起動しています…' : `停止中${since ? ` · ${since}` : ''}`,
    stopped: true,
  };
}

/* What waits on the person on a board: what the server counts, and the sessions waiting on a
   permission prompt or a question, which it does not. Not stored in `b.waiting`: that is
   lowered locally as gates are answered. */
const boardWaiting = b => (b.waiting || 0) + (b.waits || []).length;

function boardRowHtml(b, { child, waiting, working, chevron, folded, own = waiting }) {
  const st = boardState(b);
  const current = nav.board === b.slug && nav.view !== 'review' && nav.view !== 'work';
  const sub = st.stopped ? st.text : (b.hub ? '親タスク hub' : `${(b.nwo || '').split('/')[0]} · リポジトリ hub`);
  const start = st.stopped
    ? `<button type="button" class="start-btn" data-start-slug="${esc(b.slug)}" title="hub を起動します" ${rowStartingNow(b) ? 'disabled' : ''}>起動</button>` : '';
  const term = st.stopped ? ''
    : `<button type="button" class="hubterm-btn" data-hub-term="${esc(b.slug)}" title="この hub を開く" aria-label="この hub を開く"><span class="material-symbols-outlined" aria-hidden="true">terminal</span></button>`;
  const toggle = chevron
    ? `<button type="button" class="repo-toggle" data-fold="${esc(b.nwo)}" aria-expanded="${!folded}" title="${folded ? 'hub を開く' : 'hub をたたむ'}" aria-label="${folded ? 'hub を開く' : 'hub をたたむ'}"><span class="material-symbols-outlined" aria-hidden="true">expand_more</span></button>` : '';
  return `<div class="board-row ${child ? 'child' : 'repo'}${st.stopped ? ' stopped' : ' live'}" role="link" tabindex="0" data-board="${esc(b.slug)}"${current ? ' aria-current="page"' : ''} title="${esc(boardName(b))}">
    <span class="avatar" aria-hidden="true">${esc(initialsOf(b))}<span class="mini-badge">${own || ''}</span></span>
    <span class="hub-dot ${st.tone}" title="${esc(st.stopped ? '停止中' : '稼働中')}"></span>
    <span class="txt"><span class="name">${esc(boardName(b))}</span><span class="sub">${esc(sub)}</span></span>
    <span class="counts"><span class="rail-badge${waiting ? '' : ' zero'}" title="あなたの確認待ち">${waiting}</span><span class="rail-count${working ? '' : ' zero'}" title="作業中の worker">${working}</span></span>
    ${start}${term}${toggle}</div>`;
}

/* Boards as the sidebar lists them: repositories by name, each with its own board and then its
   parent-task hubs by name. The review queue groups its items the same way. */
function repoGroups(list) {
  const repos = [];
  for (const b of list) {
    let group = repos.find(r => r.nwo === b.nwo);
    if (!group) repos.push(group = { nwo: b.nwo, repo: null, children: [] });
    if (b.hub) group.children.push(b); else group.repo = b;
  }
  repos.sort((a, b) => a.nwo.localeCompare(b.nwo));
  for (const r of repos) r.children.sort((a, b) => (a.hub || '').localeCompare(b.hub || ''));
  return repos;
}
const orderedBoards = list => repoGroups(list).flatMap(r => [...(r.repo ? [r.repo] : []), ...r.children]);

function renderBoardRows() {
  const box = document.getElementById('board-rows');
  const sub = document.getElementById('boards-sub');
  if (!multiBoard) {
    if (sub) sub.textContent = repoName() || '';
    if (box) box.innerHTML = '';
    return;
  }
  if (!box) return;
  // A parent-task hub that is finished is out of the list, unless it is the one being read.
  const shown = boards.filter(b => !b.finished || nav.board === b.slug);
  const repos = repoGroups(shown);
  const live = boards.filter(b => !b.finished);
  const total = live.reduce((n, b) => n + boardWaiting(b), 0);
  const working = live.reduce((n, b) => n + (b.working || 0), 0);
  const allCurrent = scopeAll() && nav.view !== 'review';
  let html = `<div class="board-row all" role="link" tabindex="0" data-board="all"${allCurrent ? ' aria-current="page"' : ''} title="すべてのボード">
    <span class="avatar" aria-hidden="true"><span class="material-symbols-outlined" style="font-size:18px;">dashboard</span><span class="mini-badge">${total || ''}</span></span>
    <span class="txt"><span class="name">すべて</span><span class="sub">${repos.length} リポジトリ</span></span>
    <span class="counts"><span class="rail-badge${total ? '' : ' zero'}" title="あなたの確認待ち">${total}</span><span class="rail-count${working ? '' : ' zero'}" title="作業中の worker">${working}</span></span></div>`;
  for (const r of repos) {
    const folded = prefs.boardsFolded.includes(r.nwo);
    const own = r.repo;
    const withChildren = r.children.length > 0;
    let rows = '';
    if (own) {
      const hidden = folded && withChildren;
      const waiting = boardWaiting(own) + (hidden ? r.children.reduce((n, c) => n + boardWaiting(c), 0) : 0);
      const busy = (own.working || 0) + (hidden ? r.children.reduce((n, c) => n + (c.working || 0), 0) : 0);
      rows += boardRowHtml(own, { child: false, waiting, working: busy, chevron: withChildren, folded, own: boardWaiting(own) });
    } else {
      // Parent-task hubs whose repository has no board of its own: a header with no page.
      rows += `<div class="board-row repo unlinked" title="${esc(r.nwo)}">
        <span class="avatar" aria-hidden="true">${esc((r.nwo.split('/').pop() || '').slice(0, 2).toUpperCase())}</span>
        <span class="txt"><span class="name">${esc(r.nwo.split('/').pop())}</span><span class="sub">${esc(r.nwo.split('/')[0])}</span></span>
        <button type="button" class="repo-toggle" data-fold="${esc(r.nwo)}" aria-expanded="${!folded}" title="${folded ? 'hub を開く' : 'hub をたたむ'}" aria-label="${folded ? 'hub を開く' : 'hub をたたむ'}"><span class="material-symbols-outlined" aria-hidden="true">expand_more</span></button></div>`;
    }
    for (const c of r.children) {
      rows += boardRowHtml(c, { child: true, waiting: boardWaiting(c), working: c.working || 0 });
    }
    html += `<div class="repo-group${folded && withChildren ? ' collapsed' : ''}">${rows}</div>`;
  }
  const sig = html;
  const subText = `${repos.length} リポジトリ · hub ${live.filter(b => b.hubPresent).length}`;
  if (sub) sub.textContent = subText;
  // Not rebuilt for the same rows: that would drop the focus and a click in progress.
  if (box.dataset.sig === sig) return;
  box.dataset.sig = sig;
  box.innerHTML = html;
}

document.getElementById('board-rows').addEventListener('click', e => {
  const term = e.target.closest('button[data-hub-term]');
  if (term) {
    e.stopPropagation();
    const b = boards.find(x => x.slug === term.dataset.hubTerm);
    if (b) openHubPanelOf(b);
    return;
  }
  const start = e.target.closest('button[data-start-slug]');
  if (start) {
    e.stopPropagation();
    const b = boards.find(x => x.slug === start.dataset.startSlug);
    if (b) hubStartAt(`/b/${b.slug}`, b.hubId, b.hub, b);
    return;
  }
  const fold = e.target.closest('button[data-fold]');
  if (fold) {
    const nwo = fold.dataset.fold;
    const at = prefs.boardsFolded.indexOf(nwo);
    if (at >= 0) prefs.boardsFolded.splice(at, 1); else prefs.boardsFolded.push(nwo);
    savePrefs();
    renderBoardRows();
    return;
  }
  const row = e.target.closest('.board-row[data-board]');
  if (row) openBoard(row.dataset.board);
});
document.getElementById('board-rows').addEventListener('keydown', e => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  const row = e.target.closest('.board-row[data-board]');
  if (!row || e.target !== row) return;
  e.preventDefault();
  openBoard(row.dataset.board);
});

/* A row of the sidebar: that board, in the view that is open. The review queue is not a view of
   a board, so it opens the board's cards. */
function openBoard(slug) {
  const keep = nav.view === 'review' || nav.view === 'work' ? 'agent' : nav.view;
  go({ board: slug, view: keep, task: null, item: null });
}

/* A board's hub in the task panel, on its terminal: in place when this page's state lists the
   hub, else on the board it belongs to, where the address opens it after the first poll. */
function openHubPanelOf(b) {
  const ref = HUB_REF + b.hubId;
  const listed = !scopeAll() && nav.view !== 'review' && nav.view !== 'work' && (state.hubs || []).some(h => h.id === b.hubId && h.slug === b.slug);
  if (listed) return openTaskPanel(ref, 'term');
  go({ board: b.slug, view: nav.view === 'review' || nav.view === 'work' ? 'agent' : nav.view, task: ref, pane: 'term' });
}

/* The hub of the board on screen, which the title's 「hub」 button opens. */
const ownHubId = () => selectedBoard()?.hubId || pageHub()?.id || 'hub';

/* The hub panel draws from client-side markers (starting, resetting) as well as from state, which
   change without a poll. */
function redrawHubPanel() { if (isHubRef(selectedTaskId)) renderTaskPanel(); }

function openOwnHub() { openTaskPanel(HUB_REF + ownHubId(), 'term'); }

/* The buttons that open a hub's panel are lit while it is open. */
function markHubButtons() {
  const btn = document.getElementById('btn-hub');
  if (btn) btn.classList.toggle('on', !!selectedTaskId && selectedTaskId === HUB_REF + ownHubId());
  for (const el of document.querySelectorAll('#sess-groups .sess-head-btn[data-hub-ref]')) {
    el.classList.toggle('on', !!selectedTaskId && el.dataset.hubRef === selectedTaskId);
  }
}

/* The name and state the top bar gives the screen. Only a board's own screen has its name in
   the title; the other views keep the title `setView` gave them and add the path to it. */
function renderTitle() {
  // The hub's own buttons have no board to act on in 「すべて」 and the review queue.
  const resync = document.getElementById('btn-resync');
  if (resync) resync.hidden = scopeAll() || view === 'work';
  const hubBtn = document.getElementById('btn-hub');
  if (hubBtn) hubBtn.hidden = scopeAll() || view === 'review' || view === 'work';
  markHubButtons();
  const sum = multiBoard ? boards.filter(b => !b.finished).reduce((n, b) => n + boardWaiting(b), 0) : waitingIn() + (state.waits || []).length;
  document.title = (sum ? `(${sum}) ` : '') + boardTitle();
  const crumbs = document.getElementById('crumbs');
  const title = document.getElementById('page-title');
  const subtitle = document.getElementById('page-subtitle');
  if (!crumbs) return;
  const entry = selectedBoard();
  const sep = '<span class="sep">›</span>';
  let trail;
  if (view === 'review') {
    trail = ['全体', '要対応'];
  } else if (view === 'work') {
    trail = ['全体', 'いまの仕事'];
  } else if (scopeAll()) {
    trail = ['すべてのボード'];
  } else if (entry) {
    const [owner] = (entry.nwo || '').split('/');
    trail = entry.hub ? [owner, repoNameOf(entry), boardName(entry)] : [owner, repoNameOf(entry)];
  } else {
    const [owner] = (state.repo || '').split('/');
    trail = owner ? [owner, repoName()] : [];
  }
  crumbs.innerHTML = trail.map(esc).join(sep);
  // The セッション tab is a view of the board, under the board's own title.
  const ofBoard = view === 'board' || view === 'sessions';
  if (subtitle) subtitle.hidden = ofBoard;
  if (!ofBoard || !title) return;
  if (scopeAll()) { title.textContent = 'すべて'; return; }
  const hubState = entry ? boardState(entry)
    : (state.hub?.present ? { tone: 'good' } : { tone: state.hub?.stale ? 'bad' : 'off' });
  const present = entry ? !!entry.hubPresent : !!state.hub?.present;
  const name = entry ? boardName(entry) : (pageHub()?.parent ? (pageHub().key || '') : (repoName() || 'タスクボード'));
  title.innerHTML = `${esc(name)} <span class="hub-pill ${present ? 'good' : hubState.tone === 'bad' ? 'bad' : 'off'}">${present ? 'hub 稼働中' : 'hub 停止中'}</span>`;
}

/* Start the hub of the board at `base`. The route is the resident's, so it works from any
   page. */
async function hubStartAt(base, hubId, key, row = null) {
  if (row && rowStartingNow(row)) return note('hub を起動', false, 'hub を起動しています');
  const line = key ? `adj hub --tab --hub=${key}` : 'adj hub --tab';
  try {
    const data = await boardApi(base, `/api/hubs/${encodeURIComponent(hubId)}/start`, { method: 'POST', body: '{}' });
    if (row && !data.alreadyRunning) rowStartedAt[row.slug] = Date.now();
    note(line, false, data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました');
    await refreshBoards();
    renderBoardRows();
    redrawHubPanel();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

async function hubStart(id) {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  // A start now would resume the old conversation, or open a second window.
  if (hubStartingNow(h)) return note('hub を起動', false, 'hub を起動しています');
  await hubStartAt(BASE, id, h.key);
  await refresh();
}

let hubStopTarget = null;
let hubStopMode = 'stop';
/* One dialog for all four: `mode` is 'stop' for a hub that keeps its place in the list,
   'close' for a parent-task hub that leaves it, 'reset' for one that is started again on
   a new conversation, and 'restart' for one that is started again on the same one. */
function openHubStopDialog(id, mode = 'stop') {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  if (hubTurnover(h)) return note(`${hubTurnover(h)}。終わってから操作してください`, true);
  if (mode === 'close' && hubStartingNow(h)) return note('hub を起動しています。起動してから閉じてください', true);
  hubStopTarget = id;
  hubStopMode = mode;
  const closing = mode === 'close';
  const resetting = mode === 'reset';
  const restarting = mode === 'restart';
  const running = h.state?.present;
  const label = restarting ? 'セッションを再起動' : resetting ? 'hub をリセット' : closing ? 'hub を閉じる' : 'hub を止める';
  document.getElementById('hub-stop-heading').textContent = label;
  document.getElementById('hub-stop-confirm').textContent = label;
  document.getElementById('hub-stop-icon').textContent = restarting ? 'autorenew' : resetting ? 'fiber_new' : 'stop_circle';
  // A restart is not a loss, so it does not wear the red of the others.
  document.getElementById('hub-stop-submit').className = restarting ? 'btn-m3-primary' : 'btn-m3-danger';
  const hubSession = (state.sessions || []).find(x => x.kind === 'hub' && x.id === h.id);
  fillWarnings('hub-stop-warnings', restarting && hubSession ? restartWarnings(hubSession) : []);
  document.getElementById('hub-stop-lead').textContent = restarting
    ? `${h.name} を止めて、同じ会話で起動し直します（adj hub --resume${h.key ? ` --hub ${h.key}` : ''} と同じです）。`
    : resetting
    ? `${h.name} を新しい会話で起動し直します（adj hub --new${h.key ? ` --hub ${h.key}` : ''} と同じです）。${running ? '動いている tmux のペインを閉じて hub を止めてから起動します。' : ''}`
    : closing
      ? `${h.name} を閉じます。${running ? '動いている tmux のペインを閉じて hub を止め、' : ''}一覧から外します。`
      : `${h.name} が動いている tmux のペインを閉じます。hub は止まります。`;
  document.getElementById('hub-stop-note').textContent = restarting
    ? `受信箱・タスク・gate の記録と動いている worker はそのまま残ります。起動し直した hub は受信箱を読み直して続けます${h.inboxCount ? `（未読 ${h.inboxCount} 件）` : ''}。`
    : resetting
    ? '今の会話は再開されません（会話そのものは消えませんが、このあと adj hub --resume で戻るのは新しい会話です。会話を記録しない runner では戻り先がなくなります）。受信箱・タスク・gate の記録と動いている worker はそのまま残り、新しい hub が起動時に受信箱を処理します。'
    : closing
      ? 'このハブを閉じて一覧から外します。タスク・gate・受信箱の記録は残り、同じキーで起動すると引き継ぎます。'
      : '受信箱・タスク・gate の記録は残ります。次に起動すると、hubAutoResumeHours 以内なら同じ会話を再開します。';
  const dialog = document.getElementById('hub-stop-dialog');
  dialog.returnValue = '';
  dialog.showModal();
}
document.getElementById('hub-stop-dialog').addEventListener('close', e => {
  const id = hubStopTarget;
  hubStopTarget = null;
  if (e.target.returnValue === 'stop' && id) {
    if (hubStopMode === 'close') hubClose(id);
    else if (hubStopMode === 'reset') hubReset(id);
    else if (hubStopMode === 'restart') hubRestart(id);
    else hubStop(id);
  }
});

/* Stop the hub if it runs and start it on a new conversation, as `adj hub --new` does. The
   starting markers are the ones a start from the + menu sets, so no start button is offered
   while the new window comes up. */
async function hubReset(id) {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  if (hubTurnover(h)) return note(`${hubTurnover(h)}。終わってから操作してください`, true);
  const line = `adj hub --tab --new${h.key ? ` --hub=${h.key}` : ''}`;
  const run = async () => {
    const data = await api(`/api/hubs/${encodeURIComponent(id)}/reset`, { method: 'POST', body: '{}' });
    if (!data.alreadyRunning) {
      if (h.parent) { if (h.key) hubKeyStartedAt[h.key] = Date.now(); } else repoHubStartedAt = Date.now();
    }
    return data;
  };
  const inSessions = view === 'sessions';
  const body = async () => {
    // From here until the answer the hub counts as starting: the request spans the stop and
    // the window opening, and a poll in between sees a stopped hub whose start button would
    // resume the old conversation. Added and removed in this one function, so that no early
    // return can leave the hub marked.
    hubResetting.add(id);
    renderBoardRows();
    redrawHubPanel();
    if (inSessions) showSessNotice('hub をリセットしています…');
    try {
      const data = await run();
      const text = data.reset === false
        ? (data.wasRunning ? 'hub を止めましたが、別の起動が先に hub を立ち上げたため新しい会話にはなっていません' : 'hub はすでに動いているため、リセットしませんでした')
        : data.alreadyRunning ? 'hub はすでに動いています' : '新しい会話で hub を起動しました';
      note(line, false, text);
      if (inSessions) showSessNotice(text);
      if (selectedTaskId === HUB_REF + id) panelTerm.reconnect = true;
      // Inside the try and before the id is released: the redraw that follows must see the
      // state after the reset, not the one the old hub left.
      await refresh();
    } catch (e) {
      note(`${line} → ${e.message}`, true);
      if (inSessions) showSessNotice(`hub をリセットできませんでした: ${e.message}`, true);
      // The hub may have been stopped before the start failed.
      await refresh();
    } finally {
      hubResetting.delete(id);
      // `refresh` draws nothing when the state is unchanged, so the buttons are redrawn here.
      renderBoardRows();
      redrawHubPanel();
      if (inSessions) renderSessionsView();
    }
  };
  if (inSessions) return sessAct(`hub-reset-${id}`, 'hub をリセット', body);
  return body();
}

/* Stop the hub and start it again on the same conversation, as `adj hub --resume` does. Like
   `hubReset`, the hub counts as starting from the request until the new process shows, so no
   start button is offered that would open a second window. */
async function hubRestart(id) {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  if (hubTurnover(h)) return note(`${hubTurnover(h)}。終わってから操作してください`, true);
  const line = `adj hub --tab --resume${h.key ? ` --hub=${h.key}` : ''}`;
  const inSessions = view === 'sessions';
  const body = async () => {
    // Set here and dropped only on failure: on success it stays until the new hub shows (see
    // `restartPending`), because the answer comes when its window opens, not when it is up.
    hubRestarting.set(id, { pid: h.state?.pid ?? null, at: Date.now() });
    renderBoardRows();
    redrawHubPanel();
    if (inSessions) showSessNotice('hub を再起動しています…');
    try {
      const data = await api(`/api/hubs/${encodeURIComponent(id)}/restart`, { method: 'POST', body: '{}' });
      if (data.restarted === false) hubRestarting.delete(id);
      else hubRestarting.set(id, { pid: h.state?.pid ?? null, at: Date.now() });
      const text = data.restarted === false
        ? (data.wasRunning ? 'hub を止めましたが、別の起動が先に hub を立ち上げたため再起動にはなっていません' : 'hub はすでに動いているため、再起動しませんでした')
        : '同じ会話で hub を再起動しました';
      note(line, false, text);
      if (inSessions) showSessNotice(text);
      if (panelTerm.sessionId === id) panelTerm.reconnect = true;
      await refresh();
    } catch (e) {
      hubRestarting.delete(id);
      note(`${line} → ${e.message}`, true);
      if (inSessions) showSessNotice(`セッションを再起動できませんでした: ${e.message}`, true);
      // The hub may have been stopped before the start failed.
      await refresh();
    } finally {
      renderBoardRows();
      redrawHubPanel();
      if (inSessions) renderSessionsView();
    }
  };
  if (inSessions) return sessAct(`hub-restart-${id}`, 'セッションを再起動', body);
  return body();
}

async function hubStop(id) {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  if (hubTurnover(h)) return note(`${hubTurnover(h)}。終わってから操作してください`, true);
  const line = `tmux kill-pane (hub ${h.name})`;
  try {
    const data = await api(`/api/hubs/${encodeURIComponent(id)}/stop`, { method: 'POST', body: '{}' });
    clearHubStarting(h);
    redrawHubPanel();
    note(line, false, data.wasRunning ? 'hub を止めました' : 'hub はすでに止まっていました（記録を片付けました）');
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
    if (view === 'sessions') showSessNotice(`hub を止められませんでした: ${e.message}`, true);
  }
}

async function hubClose(id) {
  const h = (state.hubs || []).find(x => x.id === id);
  if (!h) return;
  if (hubTurnover(h)) return note(`${hubTurnover(h)}。終わってから操作してください`, true);
  if (hubStartingNow(h)) return note('hub を起動しています。起動してから閉じてください', true);
  const line = `adj hub-close (hub ${h.name})`;
  try {
    const data = await api(`/api/hubs/${encodeURIComponent(id)}/close`, { method: 'POST', body: '{}' });
    clearHubStarting(h);
    redrawHubPanel();
    const unread = data.unread ? `（未読 ${data.unread} 件は残っています）` : '';
    note(line, false, `hub を閉じました${unread}`);
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
    if (view === 'sessions') showSessNotice(`hub を閉じられませんでした: ${e.message}`, true);
  }
}

function openHandoverDialog(id, before = null) {
  const task = (state.tasks || []).find(t => t.id === id);
  if (!task) return;
  handoverTargetTaskId = id;
  handoverTargetBefore = before;
  const dialog = document.getElementById('handover-dialog');
  const titleEl = document.getElementById('handover-task-title');
  const textarea = document.getElementById('handover-instruction');
  if (titleEl) titleEl.textContent = task.title;
  if (textarea) {
    textarea.value = task.instruction || '';
    textarea.onkeydown = (e) => {
      // keyCode 229 too: where compositionend comes first, the keydown that confirms the IME
      // text already has isComposing false.
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
        e.preventDefault();
        submitHandover();
      }
    };
  }
  if (dialog) {
    dialog.showModal();
    if (textarea) setTimeout(() => textarea.focus(), 50);
  }
}

function closeHandoverDialog() {
  const dialog = document.getElementById('handover-dialog');
  if (dialog && dialog.open) dialog.close();
  handoverTargetTaskId = null;
  handoverTargetBefore = null;
}

let handoverBusy = false;
async function submitHandover(e) {
  if (e) e.preventDefault();
  // A second press while the first is still being handled would hand the task over twice.
  if (!handoverTargetTaskId || handoverBusy) return;
  handoverBusy = true;
  const submit = document.getElementById('handover-submit-btn');
  if (submit) submit.disabled = true;
  try {
    await submitHandoverNow();
  } finally {
    handoverBusy = false;
    if (submit) submit.disabled = false;
  }
}
document.getElementById('handover-form').addEventListener('submit', submitHandover);

async function submitHandoverNow() {
  const textarea = document.getElementById('handover-instruction');
  const instruction = textarea ? textarea.value.trim() : '';
  const id = handoverTargetTaskId;
  const before = handoverTargetBefore;
  closeHandoverDialog();
  // Dropped onto a card: the place is made in the queue just before it is handed over.
  const order = await queueOrder(id, before);
  if (order == null) return;
  hand(id, instruction, order);
}

async function hand(id, instruction = null, order = null) {
  // Handed over without a place in mind, the task joins the end of the queue. Its own order
  // dates from when it was created, which would put it ahead of tasks handed over before it.
  if (order == null) order = await queueOrder(id, null);
  const body = { status:'queued' };
  // A string came from a field the person saw, so an empty one clears the instruction kept on
  // the task. Null is a hand-over with no field on screen, which leaves it as it was.
  if (instruction != null) body.instruction = instruction.trim();
  if (order != null) {
    body.order = order;
  }
  const cmd = body.instruction ? `adj task update --id ${id} --status queued --instruction -`
    : body.instruction === '' ? `adj task update --id ${id} --status queued --instruction ''`
    : `adj task update --id ${id} --status queued`;
  await update(id, body, cmd,
    'レコードを queued にして、受信箱に kind:request を配置し hubWake を実行します');
}

async function update(id, body, line, why) {
  try {
    const data = await api(`/api/tasks/${encodeURIComponent(id)}`, {
      method:'POST', body: JSON.stringify(body),
    });
    note(line, false, why + handedNote(data.handed));
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

async function nudgeHub(base = BASE) {
  const line = "adj send --kind next --from dashboard --subject 'start the next queued task if a worker slot is free'";
  try {
    const data = await boardApi(base, '/api/hub/next', { method: 'POST' });
    note(line, false, '枠が空いていれば待ちの先頭を着手' + handedNote(data.handed));
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

/* The hub entry's wake button (renderHubStrip). It types the hub's wake line and leaves it no
   message. `busy` (by the hub's slug, so a press on one board leaves another's button alone) keeps a second press from sending another while one is on its way; `why` is
   what the last press that typed nothing was told, by the hub's slug, kept until the next press
   or until nothing is left to read. */
const hubWake = { busy: {}, why: {} };

async function wakeHub() {
  const h = pageHub();
  // The button's disabled state is the rule; this repeats it for a click that reaches here anyway.
  if (!h || hubWake.busy[h.slug] || !((h.unseen || 0) > 0 && h.state?.present)) return;
  const slug = h.slug;
  const epoch = navEpoch;
  const line = 'hub を起こす（POST /api/hub/wake）';
  // Focus on the button goes to the next control while it is disabled, and back when it is not.
  const refocus = document.activeElement?.dataset?.action === 'wake-hub';
  hubWake.busy[slug] = true;
  delete hubWake.why[slug];
  renderHubStrip();
  try {
    const data = await boardApi(BASE, '/api/hub/wake', { method: 'POST' });
    if (data.woken) note(line, false, 'hub の端末に入力しました');
    else {
      // A refused wake is an answer, and the page says why next to the button.
      hubWake.why[slug] = data.present ? `入力しませんでした: ${data.why || '理由不明'}` : 'hub は停止中のため入力しませんでした';
      note(line, true, hubWake.why[slug]);
    }
  } catch (e) {
    hubWake.why[slug] = `入力できませんでした: ${e.message}`;
    note(`${line} → ${e.message}`, true);
  } finally {
    delete hubWake.busy[slug];
    renderHubStrip();
    // Only where the strip put it: a person who has moved on is left where they are.
    if (refocus && document.activeElement?.dataset?.action === 'hub-strip-open') {
      [...document.querySelectorAll('#hub-strip button')].find(b => b.dataset.action === 'wake-hub' && !b.disabled)?.focus();
    }
  }
  // The board moved on while this was on its way: its answer is not this one's to refresh.
  if (epoch === navEpoch) await refresh();
}

/* Read a task's issue again. On a click only: the board never asks the tracker on its own. */
async function fetchIssue(id, e) {
  const line = `adj task fetch-issue --id ${id}`;
  const button = e?.currentTarget;
  if (button) button.disabled = true;
  try {
    await api(`/api/tasks/${encodeURIComponent(id)}/issue`, { method: 'POST' });
    note(line, false, 'Issue を取得しました');
    await refresh();
  } catch (err) {
    note(`${line} → ${err.message}`, true);
  } finally {
    if (button) button.disabled = false;
  }
}

async function refreshPrs(e) {
  const line = 'adj task refresh';
  const button = e?.currentTarget;
  if (button) button.disabled = true;
  try {
    const data = await api('/api/refresh', { method: 'POST' });
    const left = [['open', 'open'], ['closed', '閉じた'], ['unreadable', '読めない'], ['skipped', '途中で変更'], ['failed', '移せなかった']]
      .filter(([k]) => data[k]?.length)
      .map(([k, label]) => `${label} ${data[k].length}`);
    const moved = data.done.length ? `${data.done.length} 件を完了に` : '完了に移すものは無し';
    note(line, false, moved + (left.length ? ` / 触らなかった: ${left.join('・')}` : ''));
    // One line for all of them: the log keeps five, and the summary above must not scroll off.
    if (data.unreadable?.length) {
      note(`${line}: 状態を読めなかった PR`, true, data.unreadable.map(t => `${t.pr} (${t.error})`).join(' / '));
    }
    if (data.failed?.length) {
      note(`${line}: マージ済みだが完了に移せなかったタスク`, true, data.failed.map(t => `${t.id} (${t.error})`).join(' / '));
    }
    await refresh();
  } catch (err) {
    note(`${line} → ${err.message}`, true);
  } finally {
    if (button) button.disabled = false;
  }
}

function handedNote(handed) {
  if (!handed) return '';
  if (!handed.present) return ' / hub は停止中のため、次回起動時に処理されます';
  return handed.woken ? ' / hub を起動/通知しました' : ' / hub は稼働中。次回の受信箱確認時に処理されます';
}

function openForm() {
  if (noBoard() && !boards.some(b => !b.finished)) return note('新しいタスク', true, '作り先のボードがありません');
  // 「すべて」 has no board of its own to create the task in: ask which.
  const field = document.getElementById('f-board');
  if (field) {
    field.hidden = !noBoard();
    if (noBoard()) {
      field.querySelector('select').innerHTML = boards.filter(b => !b.finished)
        .map(b => `<option value="${esc(b.slug)}">${esc(b.hub ? `${repoNameOf(b)} › ${boardName(b)}` : repoNameOf(b))}</option>`).join('');
    }
  }
  document.getElementById('form').showModal();
  syncForm();
}
/* The new-task form for a child of the parent at `parentUrl`: a task that is filed first and then
   started, with the parent filled in, and the details open so that it can be seen. `slug` is the board
   the parent's task is on, which 「すべて」 would otherwise ask about. */
function openChildForm(parentUrl, slug) {
  openForm();
  const form = document.getElementById('form');
  if (!form.open) return;
  // An entry left from an earlier time would be sent with this child.
  form.querySelector('form').reset();
  const kind = form.querySelector('input[name=kind][value=file-and-start]');
  if (kind) kind.checked = true;
  form.querySelector('input[name=parent]').value = parentUrl;
  form.querySelector('details.more').open = true;
  const board = form.querySelector('#f-board select');
  if (slug && board && [...board.options].some(o => o.value === slug)) board.value = slug;
  syncForm();
}
function syncForm() {
  const kind = document.querySelector('input[name=kind]:checked').value;
  document.getElementById('f-issue').classList.toggle('hidden', kind !== 'start');
  document.getElementById('f-wtname').classList.toggle('hidden', kind === 'start');
  // An Issue URL alone is enough to hand an issue over: the server reads its title and body.
  const issueUrl = document.querySelector('#f-issue input[name=issueUrl]').value.trim();
  const isIssue = /^https?:\/\/[^/]+\/[^/]+\/[^/]+\/issues\/\d+\/?(?:[?#].*)?$/.test(issueUrl);
  document.querySelector('textarea[name=body]').required = !(kind === 'start' && isIssue);
}

async function submitForm(e) {
  const f = new FormData(e.target);
  const status = e.submitter?.value || 'backlog';
  const titleText = (f.get('title') || '').trim();
  const bodyText = (f.get('body') || '').trim();
  const body = {
    body: bodyText,
    kind: f.get('kind'),
    doneWhen: f.get('doneWhen'),
    stopAt: f.get('stopAt'),
    executor: f.get('executor') || 'worker',
    autoStart: f.get('autoStart') === 'true',
    status,
  };
  if (titleText) body.title = titleText;
  for (const key of ['issueUrl', 'base', 'parent', 'worktreeName']) {
    const value = (f.get(key) || '').trim();
    if (value) body[key] = value;
  }
  const line = `adj task add` + (titleText ? ` --title '${titleText}'` : '') + ` --kind ${body.kind}` +
    (body.stopAt && body.stopAt !== 'plan' ? ` --stop-at ${body.stopAt}` : '') +
    (body.executor === 'jules' ? ' --executor jules' : '') + (status === 'queued' ? ' --queue' : '');
  if (noBoard() && !f.get('board')) return note('adj task add', true, '作り先のボードを選んでください');
  try {
    const into = noBoard() ? `/b/${f.get('board')}` : BASE;
    const data = await boardApi(into, '/api/tasks', { method:'POST', body: JSON.stringify(body) });
    note(line, false, (status === 'queued' ? '記録して受信箱へ' : 'Backlog は受信箱へ送信しません') + handedNote(data.handed) +
      (data.task?.titlePending ? '。Issue を読めませんでした。タイトルは着手時に取得します' : ''));
    e.target.reset();
    syncForm();
    await refresh(true);
  } catch (err) {
    note(`${line} → ${err.message}`, true);
  }
}
document.querySelector('#form form').addEventListener('submit', submitForm);
for (const r of document.querySelectorAll('#form input[name=kind]')) r.addEventListener('change', syncForm);
for (const ev of ['input', 'change']) document.querySelector('#f-issue input[name=issueUrl]').addEventListener(ev, syncForm);

function note(line, isError, why) {
  log.unshift({ line, isError, why });
  log = log.slice(0, 5);
  document.getElementById('log').innerHTML = log.map(l =>
    `<li class="${l.isError ? 'err' : ''}"><b>${l.isError ? '!' : '$'}</b> ${esc(l.line)}` +
    (l.why ? `   <span style="color:var(--muted)">— ${esc(l.why)}</span>` : '') + '</li>').join('');
  const latestCmd = document.getElementById('sheet-latest-cmd');
  if (latestCmd) {
    latestCmd.textContent = line;
    latestCmd.style.color = isError ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-outline)';
  }
}

// The ball count belongs in the tab title: you should know it is your turn without
// having to look at the page. The board's name follows it, so tabs of several boards differ.
registerView('board-rows', { render: () => renderBoardRows() });
registerView('title', { render: () => renderTitle() });
