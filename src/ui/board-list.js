/* The board as a list (the default view): one table-like grid, a section per stage in the order of the columns, and the switch
   between the list and the columns in the board's head. `renderBoard` (board.js) decides which of the two is drawn. */

const LIST_HEAD = '<div class="list-head" aria-hidden="true"><span>待ち時間</span><span>スタック</span><span>キー</span><span>タイトル</span><span class="c-parent">親</span><span class="c-for">待っているもの</span><span>PR・状態</span></div>';

/* The list or the columns is shown, with what goes with it in the head: the switch, and 「親Issueでまとめる」, which only the list has. */
function syncBoardHead(list) {
  const agent = document.getElementById('board-agent');
  const rows = document.getElementById('board-list');
  if (agent) agent.hidden = list;
  if (rows) rows.hidden = !list;
  const opt = document.getElementById('board-group-opt');
  if (opt) opt.hidden = !list;
  const box = document.getElementById('board-group');
  if (box) box.checked = prefs.boardGroup;
  for (const b of document.querySelectorAll('[data-board-view]')) {
    const on = b.dataset.boardView === (list ? 'list' : 'cols');
    b.classList.toggle('active', on);
    b.setAttribute('aria-pressed', String(on));
  }
}

/* The parent a row names: the Issue the board joined to the task, else the record's own text. */
function parentChipHtml(task) {
  const p = task.parentIssue;
  const name = p ? (p.number ? `#${p.number}` : p.key) : task.parent;
  if (!name) return '';
  return `<span class="origin-chip parent-chip" title="${esc(p ? p.title || p.key : task.parent)}"><span class="material-symbols-outlined" aria-hidden="true">account_tree</span><span>親: ${esc(name)}</span></span>`;
}

/* The cells of a task's row, in the order of LIST_HEAD, and whether the row waits on the person. What a row says is what the
   card says: the wait (`waitWordOf`), a permission prompt, what is stuck, the worker's phase, the queue. */
function taskRowParts(task) {
  const owner = task.ownerHub;
  const hcol = owner ? owner.humanCol : humanColOf(task);
  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;
  const stuck = stuckOf(task);
  const asking = live && !hcol ? sessionOfTask(task) : null;
  const asks = !!asking && sessionState(asking) === 'permission';
  let text = '', tone = 'list-st';
  if (hcol) { text = `${waitWordOf(task, hcol)}を待っています`; tone = 'list-wait'; }
  else if (asks) { text = permissionLabel(asking); tone = 'list-wait err'; }
  else if (stuck) { text = stuck; tone = 'list-wait err'; }
  else if (live && task.jules) text = `Jules ${julesText(task.jules)}`;
  else if (worker?.phase) text = `${PHASE_LABEL[worker.phase] || worker.phase}${!worker.present && task.status !== 'pr' ? '（停止）' : ''}`;
  else if (task.status === 'queued' && task.order != null && !owner) text = `キュー ${task.order}`;
  else if (task.status === 'backlog') text = 'Backlog';
  else if (task.status === 'done' && task.updatedAt) text = `${ago(task.updatedAt)}に完了`;
  const mins = phaseMinutes(worker);
  const st = stackOf(task);
  const dur = hcol && !owner ? waitBadgeHtml(task) : mins != null ? `<span class="list-dur">${esc(minutesLabel(mins))}経過</span>` : '';
  const { issue, pr } = ghChipPartsOf(task);
  const key = issue || `<span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>`;
  // What a screen reader hears for the row: the title, then what the cells say.
  const label = [task.title || task.id, hcol && !owner ? `${minutesLabel(waitingMinutes(task))}待ち` : '', text,
    agentLabel(agentColOf(task)), st ? `スタック ${st.at + 1}/${st.chain.length}` : ''].filter(Boolean).join('、');
  return {
    label,
    waiting: !!hcol || asks,
    cells: `<span>${dur}</span><span>${stackChipHtml(task)}</span><span class="list-key">${key}${originChip(task)}</span>`
      + `<span class="title" title="${esc(task.title)}">${esc(task.title)}${titlePendingPill(task)}</span>`
      + `<span class="c-parent">${parentChipHtml(task)}</span><span class="${tone} c-for">${esc(text)}</span>`
      + `<span class="list-state">${prTurnPill(task)}${pr}</span>`,
  };
}

/* A row is pressed from the keyboard through its title cell, which is the button (named by `label`). The row itself is not one: it
   holds other buttons (the stack chip, a session's links), and a button inside a button is not reached or read. */
function pressableRowTitle(el, label) {
  const title = el.querySelector('.title');
  if (!title) return;
  title.tabIndex = 0;
  title.setAttribute('role', 'button');
  title.setAttribute('aria-label', label);
  title.addEventListener('keydown', e => {
    if (e.target !== title || e.isComposing || (e.key !== 'Enter' && e.key !== ' ')) return;
    e.preventDefault();
    el.click();
  });
}

/* The row of a task: pressed, it opens what a card of it opens (`openCardTask`); from the keyboard, its title does. */
function taskRow(task) {
  const el = document.createElement('div');
  const { waiting, cells, label } = taskRowParts(task);
  el.className = 'list-row' + (waiting ? ' waiting' : '') + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `agent-${task.id}`;
  el.dataset.id = task.id;
  if (task._slug) el.dataset.slug = task._slug;
  if (task.ownerHub) {
    el.dataset.slug = task.ownerHub.slug;
    el.dataset.owner = '1';
  }
  el.innerHTML = cells;
  // A parent-task hub's row on a board served alone only shows, as its card does.
  if (task.ownerHub && !multiBoard) {
    el.style.cursor = 'default';
    return el;
  }
  el.onclick = e => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('a')) return;
    openCardTask(task, el);
  };
  pressableRowTitle(el, label);
  return el;
}

/* The row of a worker with no task, as its card says it: what it is, its state, and what makes it a task. */
function sessionRow(w) {
  const s = sessionOfWorker(w);
  const missing = sessionMissingTask(w);
  const name = s ? sessionKey(s) : baseName(w.worktree) || w.name || '';
  const st = s ? sessionState(s) : null;
  const last = s ? lastOutputText(s) : null;
  const el = document.createElement('div');
  el.className = 'list-row session-row';
  el.dataset.worktree = w.worktree;
  if (w._slug) el.dataset.slug = w._slug;
  const why = missing ? 'タスク ID はこのボードに見つかりません' : 'タスクのレコードがないセッション';
  el.innerHTML = '<span></span><span></span>'
    + `<span class="list-key"><span class="m3-pill pill-neutral" title="${esc(why)}">${missing ? 'タスクが見つからない' : 'タスクなし'}</span></span>`
    + `<span class="title" title="${esc(w.branch || w.worktree || '')}">${esc(name)}</span><span class="c-parent"></span>`
    + `<span class="list-st c-for">${last ? `最後の出力 ${esc(last)}` : ''}</span>`
    + `<span class="list-state">${st ? `<span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(ROW_LABEL[st] || STATE_LABEL[st] || st)}</span>`
      : `<span class="m3-pill ${w.present ? 'pill-good' : 'pill-neutral'}">${w.present ? '稼働' : '停止'}</span>`}${sessionLinkButtons(w)}</span>`;
  if (!pressableSession(w)) {
    el.style.cursor = 'default';
    return el;
  }
  el.onclick = e => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('a')) return;
    pressSession(w);
  };
  pressableRowTitle(el, [name, `タスクなし`, last ? `最後の出力 ${last}` : ''].filter(Boolean).join('、'));
  return el;
}

/* A parent's heading over its rows when 「親Issueでまとめる」 is on; a stack's parent says so. */
function parentHeadHtml(g) {
  const name = g.key ? [g.number ? `#${g.number}` : g.key, g.title].filter(Boolean).join(' ') : '親Issueなし';
  return `<div class="parent-head"><span class="material-symbols-outlined" aria-hidden="true">account_tree</span><span class="parent-name" title="${esc(name)}">${esc(name)}</span>`
    + `${g.stacked ? '<span class="wk-stack" title="子がスタックになっています">stack</span>' : ''}<span class="col-count-pill">${g.tasks.length}</span></div>`;
}

/* One stage: its head (name, count, and the buttons its column has) over its rows. */
function listStage({ def, tasks, bare, older, count }) {
  const sec = document.createElement('section');
  sec.className = 'stage';
  sec.dataset.col = def.id;
  sec.insertAdjacentHTML('beforeend', `<div class="stage-head${def.id === 'me' && count ? ' attention' : ''}"><span class="material-symbols-outlined" aria-hidden="true">${def.icon}</span><span>${esc(def.label)}</span><span class="col-count-pill">${count}</span>${nextBtnHtml(def.id)}${prBtnHtml(def.id)}</div>${prWarnHtml(def.id)}`);
  const rowsOf = ts => {
    const box = document.createElement('div');
    box.className = 'list-rows';
    for (const t of ts) box.appendChild(taskRow(t));
    return box;
  };
  if (def.id === 'me' && prefs.boardGroup && tasks.length) {
    for (const g of parentGroupsOf(tasks, state)) {
      const grp = document.createElement('div');
      grp.className = 'parent-group';
      grp.insertAdjacentHTML('beforeend', parentHeadHtml(g));
      grp.appendChild(rowsOf(g.tasks));
      sec.appendChild(grp);
    }
  } else {
    const box = rowsOf(tasks);
    for (const w of bare) box.appendChild(sessionRow(w));
    if (!tasks.length && !bare.length) box.insertAdjacentHTML('beforeend', '<div class="list-empty">なし</div>');
    sec.appendChild(box);
  }
  if (older && def.id === 'done') {
    sec.insertAdjacentHTML('beforeend', `<button type="button" class="list-more" data-more-done>${showOlderDone ? '以前の完了を畳む' : `以前の完了 ${older} 件`}</button>`);
  }
  return sec;
}

/* Draws the list afresh, at the scroll position it last had (`boardScroll`, board.js). */
function renderBoardList(stages) {
  const el = document.getElementById('board-list');
  if (!el) return;
  el.innerHTML = LIST_HEAD;
  for (const st of stages) el.appendChild(listStage(st));
  el.scrollTop = boardScroll.list;
}

// The switch and the grouping box are in the board's head, outside #boards (whose clicks board.js reads).
document.addEventListener('click', e => {
  const b = e.target.closest('[data-board-view]');
  if (b) {
    const view = b.dataset.boardView === 'cols' ? 'cols' : 'list';
    if (view === prefs.boardView) return;
    prefs.boardView = view;
    savePrefs();
    closeStackPop();
    return renderBoard();
  }
  if (e.target.closest('#boards [data-more-done]')) {
    showOlderDone = !showOlderDone;
    render();
  }
});
document.getElementById('board-group')?.addEventListener('change', e => {
  prefs.boardGroup = e.target.checked;
  savePrefs();
  renderBoard();
});
