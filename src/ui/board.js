/* What `adj phase --set` writes, in the words a card shows. */
const PHASE_LABEL = { plan:'計画', implement:'実装', 'self-review':'セルフレビュー', verify:'動作確認',
                      pr:'PR', review:'レビュー対応', report:'報告' };
const workerOf = t => t.worktree && (state.workers || []).find(w => w.worktree === t.worktree);
const minutesLabel = mins => mins < 60 ? `${mins}分` : mins < 1440 ? `${Math.floor(mins / 60)}時間` : `${Math.floor(mins / 1440)}日`;
/* Minutes since the worker entered its phase, by the server's clock so a laptop that slept
   does not make every card look stuck. */
const phaseMinutes = w => w && w.phaseAt != null && state.now != null
  ? Math.max(0, Math.floor((state.now - w.phaseAt) / 60)) : null;
const stampSecs = stamp => {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp || '');
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) / 1000 : null;
};
/* Minutes the worker has had the ball: since it entered its phase, or since a person last
   answered its gate, whichever is later. */
const workerMinutes = (task, w) => {
  const mins = phaseMinutes(w);
  const answered = stampSecs(task.gateAnsweredAt);
  if (mins == null || answered == null || state.now == null) return mins;
  return Math.min(mins, Math.max(0, Math.floor((state.now - answered) / 60)));
};

/* Why a card is stuck, or null. A badge rather than a column: moved to a column of its own,
   the card would lose the column that says where it got to. */
function stuckOf(task) {
  if (!['dispatched', 'pr'].includes(task.status) || !task.worktree) return null;
  const w = workerOf(task);
  // The worker is gone and nothing will move this card: the one thing a person must hear.
  // Not in the first two minutes, while a worker that was just dispatched is still opening.
  if (!w || !w.present) return Date.now() - updatedMs(task) < 120000 ? null : 'worker 停止';
  // Time only counts while the ball is the worker's. A card waiting on a person's answer to a
  // gate, or on reviewers once its PR is open, is not the worker being stuck — flagging those
  // would bury the ones that are.
  if (openGate(task) || task.status === 'pr') return null;
  const mins = workerMinutes(task, w);
  const limit = state.stuckAfterMinutes;
  if (mins != null && limit > 0 && mins >= limit) return `${minutesLabel(mins)} 同じ工程`;
  return null;
}

/* Done cards older than this fold away. The records stay; the column is for
   what finished recently, not an archive to scroll past. */
const DONE_SHOWN_HOURS = 24;
let showOlderDone = false;
const updatedMs = t => {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(t.updatedAt || '');
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) : Date.now();
};

/* The buttons a card has for the worker behind it. Each runs on the server through the same
   templates the commands use; the log line says which command that was. */
async function focusHub() {
  const line = 'adj focus';
  try {
    const data = await api('/api/hub/focus', { method: 'POST', body: '{}' });
    note(line, false, data.present ? (data.ran ? 'hub のタブを前に出しました' : 'hub のタブが見つかりませんでした') : 'hub は動いていません');
  } catch (e) { note(`${line} → ${e.message}`, true); }
}

/* `confirmed` is set by the close dialog: closing stops the worker, so it is asked there first. */
async function worktreeAct(action, worktree, confirmed = false) {
  const line = { focus: `adj focus --worktree ${worktree}`, ide: `adj ide --worktree ${worktree}`,
                 close: `adj close --worktree ${worktree}` }[action];
  // With no editor configured the server can only refuse, so say how to set one instead.
  if (action === 'ide' && !ideReady()) { openIdeDialog(); return; }
  if (action === 'close' && !confirmed) { openCloseDialog(worktree); return; }
  try {
    const data = await api(`/api/worktrees/${action}`, { method: 'POST', body: JSON.stringify({ worktree }) });
    const why = action === 'focus' ? (data.present ? (data.ran ? 'タブを前に出しました' : 'worker のタブが見つかりませんでした') : 'worker は動いていません')
              : action === 'close' ? (data.closed ? 'タブを閉じました' : 'まだ閉じていません（確認待ちかもしれません）')
              : 'エディタで開きました';
    note(line, false, why);
    await refresh();
  } catch (e) { note(`${line} → ${e.message}`, true); }
}

const COLUMN_ICONS = {
  backlog: 'inventory_2',
  queued: 'hourglass_top',
  working: 'smart_toy',
  attention: 'pending_actions',
  pr: 'rate_review',
  done: 'task_alt',
};
const COLUMN_SUBS = {
  backlog: '未受付',
  queued: '着手待ち',
  working: 'AI作業中',
  attention: '要判定',
  pr: 'レビュー対応',
  done: '完了済',
};

let currentActiveFilter = 'all';

function setBoardFilter(filter, btn) {
  currentActiveFilter = filter;
  document.querySelectorAll('.m3-filter-chip').forEach(b => {
    b.classList.toggle('active', b === btn);
    b.setAttribute('aria-pressed', String(b === btn));
  });
  const board = document.getElementById('board');
  if (board) {
    board.innerHTML = '';
    for (const col of COLUMNS) board.appendChild(columnEl(col));
  }
}

function columnEl(col) {
  let items = (state.tasks || []).filter(t => columnOf(t) === col.id);
  if (currentActiveFilter === 'mine') {
    items = items.filter(t => openGate(t) || col.id === 'backlog');
  } else if (currentActiveFilter === 'worker') {
    items = items.filter(t => {
      const w = workerOf(t);
      return w && w.present;
    });
  }

  let older = [];
  if (col.id === 'done') {
    const cutoff = Date.now() - DONE_SHOWN_HOURS * 3600 * 1000;
    older = items.filter(t => updatedMs(t) < cutoff);
    if (!showOlderDone) items = items.filter(t => updatedMs(t) >= cutoff);
  }
  const el = document.createElement('section');
  el.className = 'col' + (col.id === 'attention' ? ' attention' : '');
  el.dataset.col = col.id;

  for (const task of items) el.appendChild(cardEl(task, col.id));
  if (col.id === 'working' && currentActiveFilter !== 'mine') {
    const known = new Set(items.map(t => t.worktree).filter(Boolean));
    for (const w of (state.workers || []).filter(w => w.present && !known.has(w.worktree))) {
      el.appendChild(ghostEl(w));
    }
  }
  if (col.id === 'attention') {
    const shown = new Set(items.map(t => openGate(t)?.id).filter(Boolean));
    for (const g of (state.gates || []).filter(g => {
      if (shown.has(g.id)) return false;
      if (currentActiveFilter !== 'worker') return true;
      const owner = (state.tasks || []).find(t => t.id === g.task);
      return !owner || workerOf(owner)?.present;
    })) {
      el.appendChild(gateCardEl(g));
    }
  }
  const shownCards = el.querySelectorAll('.card').length;
  if (!shownCards) el.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder"><span class="material-symbols-outlined" style="font-size:16px;">inbox</span><span>タスクなし</span></div>');
  if (older.length) {
    el.insertAdjacentHTML('beforeend',
      `<button type="button" class="more">${showOlderDone ? '以前の完了を畳む' : `以前の完了 ${older.length} 件`}</button>`);
    el.querySelector('.more').addEventListener('click', () => { showOlderDone = !showOlderDone; render(); });
  }

  const iconName = COLUMN_ICONS[col.id] || 'view_kanban';
  const subText = COLUMN_SUBS[col.id] || col.hint || '';
  const nextBtn = col.id === 'queued' && shownCards
    ? '<button type="button" class="col-btn-nudge" title="workerの枠が空いていれば次を着手"><span class="material-symbols-outlined" style="font-size:14px;">bolt</span><span>次を流す</span></button>'
    : '';
  const checkBtn = col.id === 'pr'
    ? '<button type="button" class="col-btn-nudge" title="PRマージ済みタスクを確認"><span class="material-symbols-outlined" style="font-size:14px;">sync</span><span>PR確認</span></button>'
    : '';

  const headerHtml = `
    <div class="col-header">
      <div class="col-header-top">
        <div class="col-title-badge">
          <span class="material-symbols-outlined" style="font-size:18px;">${iconName}</span>
          <span>${esc(col.label)}</span>
          <span class="col-count-pill">${shownCards}</span>
        </div>
        ${nextBtn}${checkBtn}
      </div>
      ${subText ? `<div class="col-subtext">${esc(subText)}</div>` : ''}
    </div>
  `;
  el.insertAdjacentHTML('afterbegin', headerHtml);
  el.querySelector('.col-btn-nudge')?.addEventListener('click', col.id === 'pr' ? refreshPrs : nudgeHub);

  el.addEventListener('dragover', e => {
    if (canDrop(window.__from, col.id)) { e.preventDefault(); el.classList.add('drop-ok'); }
  });
  el.addEventListener('dragleave', () => el.classList.remove('drop-ok'));
  el.addEventListener('drop', e => {
    e.preventDefault();
    el.classList.remove('drop-ok');
    move(e.dataTransfer.getData('text/plain'), col.id, e.target.closest('.card')?.dataset.id);
  });
  return el;
}

const DONE_WHEN = { 'report-only':'調査のみ', verify:'動作確認まで', pr:'PR作成まで', review:'レビュー対応まで' };
const STOP_AT = { plan:'計画の承認だけ待つ', diff:'計画と差分レビューを待つ', all:'計画・差分・動作確認を待つ' };

/* The rules a worker's procedure stops a diff or verify gate by, in the words the board
   shows. The second value marks the two that mean something went wrong, rather than that a
   person was asked for. */
const STOP_RULE = {
  'round-limit':   ['レビューが上限ラウンドに達して must が残った', true],
  'verify-failed': ['verify が失敗して直せなかった', true],
  'manual-check':  ['人が見る確認がある', false],
  'unsure':        ['迷っている点がある', false],
  'stop-at':       ['タスクの止める所に含まれている', false],
};
const stopWhy = g => (g.stoppedBy || []).map(r => (STOP_RULE[r] || [r])[0]);
const stopBad = g => (g.stoppedBy || []).some(r => STOP_RULE[r]?.[1]);

/* ── records: what a worker wrote down without stopping ──
   Which ones a person has opened lives in this browser's localStorage. It is one person's
   reading, not the task's state: writing it to the server would give a second reader's
   board a dot they never cleared, and the worker nothing it could use. */
const SEEN_KEY = () => `adj.seenRecords.${state.repo || ''}`;
let seenCache = null, seenCacheKey = null;
function seenRecords() {
  if (seenCache && seenCacheKey === SEEN_KEY()) return seenCache;
  seenCacheKey = SEEN_KEY();
  try { seenCache = new Set(JSON.parse(localStorage.getItem(SEEN_KEY()) || '[]')); }
  catch { seenCache = new Set(); }
  return seenCache;
}
const isUnread = r => !seenRecords().has(r.id);
function markSeen(id) {
  // Read again first: another tab may have written since, and this write would drop its ids.
  seenCache = null;
  const seen = seenRecords();
  if (seen.has(id)) return;
  seen.add(id);
  // Only the ids still on the board: a finished task's records leave /api/state, and the list
  // would otherwise only grow.
  const live = new Set(allRecords().map(r => r.id));
  try { localStorage.setItem(SEEN_KEY(), JSON.stringify([...seen].filter(x => live.has(x)))); }
  catch { /* private mode or full: the dot comes back next load, nothing worse */ }
}
// Another tab on the same board opened a record: its dot goes here too.
window.addEventListener('storage', e => {
  if (e.key !== SEEN_KEY()) return;
  seenCache = null;
  if (view === 'board') render();
  // A task view marks its tabs with the same dots.
  else if (view === 'task') redrawTaskView();
});
const allRecords = () => (state.tasks || []).flatMap(t => t.records || []);
const recordById = id => allRecords().find(r => r.id === id);
/* A task's records oldest first. The server sorts by `openedAt` alone, which is to the second,
   so two records opened in the same second come in directory order; the sequence at the end of
   the id (`…-record-2`, none for the first) is the order they were claimed in. */
const recordSeq = r => +(/-record-(\d+)$/.exec(r.id)?.[1] || 1);
const recordsOf = task => [...(task.records || [])].sort((a, b) =>
  (a.openedAt || '').localeCompare(b.openedAt || '') || recordSeq(a) - recordSeq(b));

/* One record in a few words, and whether it went well: `[text, tone]`, tone being `good`,
   `bad` or ''. Tone always comes with ✓ or ✗ in the text, so colour never says it alone. */
function recordSummary(r) {
  if (r.kind === 'diff') {
    const rounds = r.reviewRounds || [];
    const openMust = (r.findings || []).filter(f => f.severity === 'must' && f.outcome === 'open').length;
    const last = rounds[rounds.length - 1];
    if (openMust) return [`レビュー ${rounds.length ? `${rounds.length}R ` : ''}✗ must 残り ${openMust}件`, 'bad'];
    if (!rounds.length) return ['レビュー', ''];
    // Converged is a last round with no must in it; anything else stopped short.
    return last.must ? [`レビュー ${rounds.length}R ✗ 未収束`, 'bad'] : [`レビュー ${rounds.length}R ✓ 収束`, 'good'];
  }
  if (r.kind === 'verify') {
    const commands = r.commands || [];
    const failed = commands.filter(c => c.result === 'fail').length;
    if (!commands.length) return ['動作確認', ''];
    return failed ? [`verify ✗ ${failed}件失敗`, 'bad'] : ['verify ✓', 'good'];
  }
  return [kindOf(r.kind)[0], ''];
}

/* The chips a card carries: the latest record of each kind, since a review done again after a
   send-back replaces the one before it, and the checks it left for a person. */
function chipsOf(task) {
  const latest = {};
  for (const r of recordsOf(task)) latest[r.kind] = r;
  const chips = [];
  for (const r of Object.values(latest)) {
    const [text, tone] = recordSummary(r);
    chips.push({ id: r.id, text: text + ((r.answers || []).length ? ' ↩' : ''), tone, unread: isUnread(r) });
    if (r.kind === 'verify' && (r.manual || []).length) {
      chips.push({ id: r.id, text: `手で見る ${r.manual.length}件`, tone: '', unread: false });
    }
  }
  return chips;
}

/* A record opens in its task's full view, on the tab for its kind and showing that one. */
function openRecord(id) {
  const r = recordById(id);
  if (r && r.task) return openTask(r.task, TAB_OF_KIND[r.kind] || 'history', id);
  focused = id;
  setView('review');
  renderReview();
}

function cardEl(task, col) {
  const el = document.createElement('div');
  const gate = openGate(task);
  el.className = 'card' + (selectedTaskId === task.id ? ' selected' : '') + (gate ? ' attention-card' : '');
  el.dataset.id = task.id;
  if ((ALLOWED[col] || []).length) {
    el.draggable = true;
    el.addEventListener('dragstart', e => {
      window.__from = col;
      e.dataTransfer.setData('text/plain', task.id);
      el.classList.add('dragging');
    });
    el.addEventListener('dragend', () => { window.__from = null; render(); });
  }

  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;

  let h = '';

  // 1. Header row: Issue / ID & Status Pill
  const doneLabel = DONE_WHEN[task.doneWhen] || task.doneWhen;
  const donePillClass = task.doneWhen === 'report-only' ? 'pill-purple'
    : task.doneWhen === 'verify' ? 'pill-warn'
    : task.doneWhen === 'review' ? 'pill-blue'
    : 'pill-neutral';

  const issueUrl = httpUrl(task.issueUrl);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : null;

  h += `
    <div class="card-header-row">
      ${issueNumber ? `
        <a href="${esc(issueUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="card-issue-link" title="GitHub Issue #${esc(issueNumber)} を開く">
          <span class="material-symbols-outlined" style="font-size:12px;">tag</span>
          <span>${esc(issueNumber)}</span>
        </a>
      ` : `
        <span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>
      `}
      ${doneLabel ? `<span class="m3-pill ${donePillClass}">${esc(doneLabel)}</span>` : ''}
    </div>
  `;

  // 2. Title
  h += `<div class="title">${esc(task.title)}</div>`;

  // 3. Gate Banner if present
  if (gate) {
    const [label] = kindOf(gate.kind);
    const why = stopWhy(gate);
    h += `
      <button type="button" class="m3-card-attention-box" style="padding:8px 10px;font-size:12px;cursor:pointer;width:100%;text-align:inherit;border:none;font:inherit;" data-gate="${esc(gate.id)}" title="クリックして判定画面を開く">
        <div style="display:flex;align-items:center;justify-content:space-between;font-weight:700;">
          <span style="display:flex;align-items:center;gap:4px;">
            <span class="material-symbols-outlined" style="font-size:15px;">pending_actions</span>
            <span>【${esc(label)}】判定待ち</span>
          </span>
          <span style="font-size:11px;opacity:.9">${ago(gate.openedAt)}</span>
        </div>
        ${why.length ? `<div style="font-size:11px;margin-top:2px;">${esc(why.join(' / '))}</div>` : ''}
      </button>
    `;
  }

  // 4. Worker Live Status
  if (worker && worker.present && worker.phase) {
    const mins = phaseMinutes(worker);
    h += `
      <div class="card-worker-status">
        <span class="pulse-dot"></span>
        <span style="font-weight:700;">${esc(PHASE_LABEL[worker.phase] || worker.phase)}</span>
        ${mins != null ? `<span style="color:var(--md-sys-color-outline);margin-left:auto;">${minutesLabel(mins)}</span>` : ''}
      </div>
    `;
  }

  // 5. Metadata tags (queued order, autoStart, PR, etc.)
  const metaBadges = [];
  if (col === 'queued' && task.order != null) {
    metaBadges.push(`<span class="m3-pill pill-neutral" title="キューの優先順"><span class="material-symbols-outlined" style="font-size:12px;">swap_vert</span>${task.order}</span>`);
  }
  if (!task.autoStart) {
    metaBadges.push(`<span class="m3-pill pill-warn" title="着手前に確認が必要"><span class="material-symbols-outlined" style="font-size:12px;">lock</span>要着手確認</span>`);
  }
  const prUrl = httpUrl(task.pr);
  if (prUrl) {
    const prNumber = prNumberOf(prUrl);
    metaBadges.push(`<a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="m3-pill pill-blue" style="text-decoration:none;" title="PRを開く (${esc(prUrl)})"><span class="material-symbols-outlined" style="font-size:12px;">merge</span>PR${prNumber ? ` #${esc(prNumber)}` : ''}</a>`);
  }
  if (metaBadges.length) {
    h += `<div style="display:flex;flex-wrap:wrap;gap:4px;">${metaBadges.join('')}</div>`;
  }

  // 6. Records / Chips
  const chips = chipsOf(task);
  if (chips.length) {
    h += `<div class="chips" style="gap:4px;">${chips.map(c => {
      const isGood = c.tone === 'good';
      const isBad = c.tone === 'bad';
      const pillClass = isGood ? 'pill-good' : isBad ? 'pill-err' : 'pill-warn';
      const icon = isGood ? 'check_circle' : isBad ? 'cancel' : 'info';
      return `<button type="button" class="m3-pill ${pillClass}" style="border:0;cursor:pointer;" data-record="${esc(c.id)}" title="${c.unread ? '未読 — ' : ''}クリックして記録を開く">
        ${c.unread ? '<span style="width:5px;height:5px;border-radius:50%;background:currentColor;display:inline-block;margin-right:2px;"></span>' : ''}
        <span class="material-symbols-outlined" style="font-size:13px;margin-right:2px;">${icon}</span>
        <span>${esc(c.text)}</span>
      </button>`;
    }).join('')}</div>`;
  }

  // 7. Note / Alert
  if (task.note) {
    if (task.status === 'done') {
      h += `<div style="font-size:11.5px;color:var(--md-sys-color-outline);overflow-wrap:anywhere;">${esc(task.note)}</div>`;
    } else {
      h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11.5px;color:var(--md-sys-color-error);font-weight:600;"><span class="material-symbols-outlined" style="font-size:14px;flex-shrink:0;">warning</span><span>${esc(task.note)}</span></div>`;
    }
  }

  // 7.5 Instruction (申し送り)
  if (task.instruction) {
    h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11px;color:var(--md-sys-color-primary);background:var(--md-sys-color-surface-container-high);padding:4px 8px;border-radius:var(--md-shape-corner-xs);margin-top:2px;">
      <span class="material-symbols-outlined" style="font-size:13px;flex-shrink:0;margin-top:1px;">forward_to_inbox</span>
      <span style="overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;" title="${esc(task.instruction)}">${esc(task.instruction)}</span>
    </div>`;
  }

  // 8. Stuck warning
  const stuck = stuckOf(task);
  if (stuck) {
    el.classList.add('stuck');
    h += `<div style="display:flex;align-items:center;gap:4px;font-size:11px;color:var(--md-sys-color-error);font-weight:700;"><span class="material-symbols-outlined" style="font-size:14px;">timer</span><span>${esc(stuck)}</span></div>`;
  }

  // 9. Footer row: slug / identifier & buttons
  const worktreeSlug = task.worktree ? task.worktree.split('/').pop() : '';
  const branchSlug = task.branch || '';

  const hasSlug = worktreeSlug || branchSlug || (!issueNumber && task.id);
  const hasButtons = task.worktree || col === 'backlog';

  if (hasSlug || hasButtons) {
    h += `
      <div class="card-footer-row">
        <div class="card-meta-slug" title="${esc(task.worktree || task.branch || task.id)}">
          ${worktreeSlug ? `<span class="material-symbols-outlined" style="font-size:13px;">folder_open</span><span>${esc(worktreeSlug)}</span>`
            : branchSlug ? `<span class="material-symbols-outlined" style="font-size:13px;">fork_right</span><span>${esc(branchSlug)}</span>`
            : issueNumber ? `<span class="card-task-id" style="font-size:10px;">${esc(task.id)}</span>`
            : ''}
        </div>
        <div class="card-button-row">
          ${task.worktree ? `<button class="m3-icon-button" title="ターミナルのworkerタブを前面表示" data-focus="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">terminal</span><span>ターミナル</span></button>` : ''}
          ${task.worktree ? `<button class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">code</span><span>IDE</span></button>` : ''}
          ${col === 'backlog' ? `<button class="m3-icon-button" style="color:var(--md-sys-color-primary);" title="待ちキューへ渡す" data-hand="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span><span>渡す</span></button>` : ''}
        </div>
      </div>
    `;
  }

  el.innerHTML = h;
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('.card-issue-link') || e.target.closest('.m3-card-attention-box')) return;
    selectTask(task.id);
  };
  el.querySelectorAll('[data-hand]').forEach(b =>
    b.addEventListener('click', (e) => { e.stopPropagation(); openHandoverDialog(b.dataset.hand); }));
  el.querySelectorAll('[data-ide]').forEach(b =>
    b.addEventListener('click', (e) => { e.stopPropagation(); worktreeAct('ide', b.dataset.ide); }));
  el.querySelectorAll('[data-focus]').forEach(b =>
    b.addEventListener('click', (e) => { e.stopPropagation(); worktreeAct('focus', b.dataset.focus); }));
  el.querySelectorAll('[data-gate]').forEach(b =>
    b.addEventListener('click', (e) => { e.stopPropagation(); judgeGate(b.dataset.gate); }));
  el.querySelectorAll('[data-record]').forEach(b =>
    b.addEventListener('click', (e) => { e.stopPropagation(); openRecord(b.dataset.record); }));
  return el;
}


function gateCardEl(gate) {
  const [label, colour] = kindOf(gate.kind);
  const el = document.createElement('div');
  el.className = 'card';
  el.style.cursor = 'pointer';
  el.innerHTML = `<div class="title">${esc(gate.title)}</div>
    <div class="meta"><span class="tag"><span class="dot" style="background:${colour}"></span>${label}</span>
      <span>${ago(gate.openedAt)}から待ち</span></div>
    <div class="mono">${esc(gate.worktree.split('/').pop())}</div>`;
  el.onclick = () => judgeGate(gate.id);
  return el;
}

function ghostEl(worker) {
  const el = document.createElement('div');
  el.className = 'card ghost';
  el.innerHTML = `<div class="title">${esc(worker.title || worker.name)}</div>
    <div class="meta"><span>${esc(worker.branch || '')}</span>
      <span class="state warn"><span>◷</span>タスクレコードなし</span></div>
    <div class="mono">${esc(worker.worktree)}</div>`;
  return el;
}

function selectTask(id) {
  selectedTaskId = (selectedTaskId === id ? null : id);
  for (const card of document.querySelectorAll('#board .card')) {
    card.classList.toggle('selected', card.dataset.id === selectedTaskId);
  }
  renderDrawer();
}

function closeDrawer() {
  selectedTaskId = null;
  for (const card of document.querySelectorAll('#board .card')) {
    card.classList.remove('selected');
  }
  renderDrawer();
}

function goToGate(gateId) {
  focused = gateId;
  setView('review');
  renderReview();
}

/* Where a waiting gate is read and answered: its task's view, in the tab of its kind, when the
   task is on the board. A gate with no task — the hub's, or one whose task is gone — has no
   such view and opens in the review view. */
function judgeGate(gateId) {
  const g = (state.gates || []).find(x => x.id === gateId);
  const owner = g?.task && (state.tasks || []).find(t => t.id === g.task);
  if (owner) openTask(owner.id, TAB_OF_KIND[g.kind] || 'history', g.id);
  else goToGate(gateId);
}

/* The レビュー tab: the first gate in the queue, where it is answered; the review view when the
   queue is empty, which says so. */
function goToQueue() {
  // Already on the queue: stay on the one being read, and on whatever is typed for it.
  if (view === 'review') return;
  const first = (state.gates || [])[0];
  if (first) judgeGate(first.id);
  else setView('review');
}

function renderDrawer() {
  const drawer = document.getElementById('task-drawer');
  if (!drawer) return;
  // Closed, not just out of view: what was typed for the task goes with it.
  if (!selectedTaskId) {
    drawer.classList.add('hidden');
    document.getElementById('drawer-body')?.replaceChildren();
    return;
  }
  if (view !== 'board') {
    drawer.classList.add('hidden');
    return;
  }
  const task = (state.tasks || []).find(t => t.id === selectedTaskId);
  if (!task) {
    drawer.classList.add('hidden');
    document.getElementById('drawer-body')?.replaceChildren();
    selectedTaskId = null;
    return;
  }
  drawer.classList.remove('hidden');

  const colId = columnOf(task);
  const colObj = COLUMNS.find(c => c.id === colId);
  const gate = openGate(task);

  const badgesEl = document.getElementById('drawer-badges');
  if (badgesEl) {
    badgesEl.innerHTML = `
      <span class="m3-pill ${gate ? 'pill-warn' : 'pill-blue'}">${colObj ? colObj.label : task.status}</span>
      <span style="font-family:var(--font-mono);font-size:11.5px;color:var(--md-sys-color-outline);margin-left:4px">${esc(task.id)}</span>
    `;
  }
  const titleEl = document.getElementById('drawer-title');
  if (titleEl) titleEl.textContent = task.title;
  const expandBtn = document.getElementById('drawer-expand-btn');
  if (expandBtn) expandBtn.onclick = () => openTask(task.id);

  let head = '';

  if (gate) {
    const [label] = kindOf(gate.kind);
    head += `
      <div class="m3-card-attention-box">
        <div style="font-weight:800;font-size:13px;display:flex;align-items:center;gap:6px;">
          <span class="material-symbols-outlined" style="font-size:16px;">pending_actions</span>
          <span>【${esc(label)}】あなたの判断待ち</span>
        </div>
        <div style="font-size:12.5px;">${esc(gate.title)}</div>
        ${stopWhy(gate).length ? `<div style="font-size:11.5px;margin-top:2px;">止めた理由: ${esc(stopWhy(gate).join(' / '))}</div>` : ''}
        <button class="btn-m3-primary" style="margin-top:6px;align-self:flex-start;" data-judge="${esc(gate.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">arrow_forward</span>
          <span>判定画面を開く</span>
        </button>
      </div>
    `;
  }

  let body = '';

  // Newest first: the one the worker left last is the one that describes where it is now.
  const records = recordsOf(task).reverse();
  if (records.length) {
    body += `<div class="m3-filled-card"><div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">記録（止めずに進んだもの）</div>` +
      records.map(r => {
        const [label] = kindOf(r.kind);
        const [text, tone] = recordSummary(r);
        const isGood = tone === 'good';
        const isBad = tone === 'bad';
        const pillClass = isGood ? 'pill-good' : isBad ? 'pill-err' : 'pill-warn';
        const icon = isGood ? 'check_circle' : isBad ? 'cancel' : 'info';
        return `<div class="d-record" style="display:flex;flex-direction:column;gap:4px;padding:8px 0;border-top:1px solid var(--md-sys-color-outline-variant);">
          <div style="display:flex;align-items:center;gap:6px;">
            <span class="m3-pill ${pillClass}">
              <span class="material-symbols-outlined" style="font-size:12px;margin-right:2px;">${icon}</span>
              <span>${esc(label)}: ${esc(text)}</span>
            </span>
          </div>
          <div style="font-size:11px;color:var(--md-sys-color-outline);">${ago(r.openedAt)}に記録</div>
          <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;" data-record="${esc(r.id)}">全体を見る・差し戻す →</button>
        </div>`;
      }).join('') + `</div>`;
  }

  const drawerPrUrl = httpUrl(task.pr);
  body += `
    <div class="m3-filled-card">
      <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">基本情報</div>
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px;font-size:12.5px;">
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">完了条件</span>
          <strong style="color:var(--md-sys-color-on-surface);">${esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')}</strong>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">確認ポイント</span>
          <strong style="color:var(--md-sys-color-on-surface);">${esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt || '—')}</strong>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">ブランチ</span>
          <code style="font-family:var(--font-mono);font-size:12px;color:var(--md-sys-color-on-surface);word-break:break-all;">${esc(task.branch || '—')}</code>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">worktree</span>
          <code style="font-family:var(--font-mono);font-size:12px;color:var(--md-sys-color-on-surface);word-break:break-all;">${esc(task.worktree ? task.worktree.split('/').pop() : '—')}</code>
        </div>
        ${drawerPrUrl ? `<div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">PR</span>
          <a href="${esc(drawerPrUrl)}" target="_blank" rel="noopener noreferrer" title="${esc(drawerPrUrl)}" style="color:var(--md-sys-color-primary);font-weight:700;text-decoration:none;display:inline-flex;align-items:center;gap:4px;"><span>${prNumberOf(drawerPrUrl) ? `#${esc(prNumberOf(drawerPrUrl))}` : 'PR を開く'}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>
        </div>` : ''}
      </div>
    </div>
  `;

  if (task.worktree) {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">開発環境の操作</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap;">
          <button class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;" data-focus="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">terminal</span>
            <span>ターミナル前面表示</span>
          </button>
          <button class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;" title="${ideTitle()}" data-ide="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">code</span>
            <span>IDE で開く</span>
          </button>
          ${workerOf(task)?.present ? `<button class="btn-m3-danger" style="padding:6px 14px;font-size:12px;" data-close="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">close</span>
            <span>タブを閉じる</span>
          </button>` : ''}
        </div>
      </div>
    `;
  }

  if (task.instruction && colId !== 'backlog') {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">エージェントへの申し送り（指示）</div>
        <p style="font-size:13px;line-height:1.6;color:var(--md-sys-color-on-surface);white-space:pre-wrap;">${esc(task.instruction)}</p>
      </div>
    `;
  }

  if (task.body) {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">依頼内容・プロンプト</div>
        <p style="font-size:13px;line-height:1.6;color:var(--md-sys-color-on-surface);white-space:pre-wrap;">${esc(task.body)}</p>
      </div>
    `;
  }

  // The latest few only: the whole history is a click away in the task view's 経過 tab.
  const all = gatesOf(task);
  body += `
    <div class="m3-filled-card" style="display:flex;flex-direction:column;">
      <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">経過</div>
      ${timelineHtml(task, all, 5)}
      <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;margin-top:6px;" data-history="${esc(task.id)}">経過をすべて見る →</button>
    </div>
  `;

  const drawerBody = document.getElementById('drawer-body');
  if (drawerBody) {
    const [headEl, formEl, restEl] = drawerParts(drawerBody);
    headEl.innerHTML = head;
    restEl.innerHTML = body;
    for (const part of [headEl, restEl]) {
      part.querySelectorAll('[data-record]').forEach(b =>
        b.addEventListener('click', () => openRecord(b.dataset.record)));
      part.querySelectorAll('[data-judge]').forEach(b =>
        b.addEventListener('click', () => judgeGate(b.dataset.judge)));
      // A gate in 経過 opens where the task view reads it, rather than being repeated here.
      part.querySelectorAll('[data-open]').forEach(b => b.addEventListener('click', () => {
        const g = all.find(x => x.id === b.dataset.open);
        if (g) openTask(task.id, TAB_OF_KIND[g.kind] || 'history', g.id);
      }));
      part.querySelectorAll('[data-history]').forEach(b =>
        b.addEventListener('click', () => openTask(b.dataset.history, 'history')));
      part.querySelectorAll('[data-focus]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('focus', b.dataset.focus)));
      part.querySelectorAll('[data-ide]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('ide', b.dataset.ide)));
      part.querySelectorAll('[data-close]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('close', b.dataset.close)));
    }
    renderHandForm(formEl, colId === 'backlog' ? task : null);
  }
}

/* The side sheet's body in three parts: what waits on the person, the hand-over form, and the
   rest. Each part is display:contents, so the body's gap still spaces the cards inside them. */
function drawerParts(drawerBody) {
  if (drawerBody.childElementCount !== 3) {
    drawerBody.innerHTML = '<div style="display:contents"></div>'.repeat(3);
  }
  return [...drawerBody.children];
}

/* The hand-over form of a backlog task. The board redraws once a minute and on every change of
   state, and a textarea built again loses what is typed into it, the caret, and an IME
   composition in progress. So the form is built again only for another task, or when the saved
   instruction changed and nothing has been typed over the one shown. */
function renderHandForm(el, task) {
  if (!task) {
    el.innerHTML = '';
    delete el.dataset.task;
    return;
  }
  const saved = task.instruction || '';
  const kept = el.querySelector('#drawer-instruction');
  if (kept && el.dataset.task === task.id && (el.dataset.saved === saved || kept.value !== el.dataset.saved)) return;
  el.dataset.task = task.id;
  el.dataset.saved = saved;
  el.innerHTML = `
      <div class="m3-filled-card" style="display:flex;flex-direction:column;gap:8px;">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;">キューへの受け渡し</div>
        <label for="drawer-instruction" style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);">エージェントへの申し送り（指示）</label>
        <textarea id="drawer-instruction" placeholder="追加の指示や申し送りがあれば入力（任意）..." style="width:100%;box-sizing:border-box;border-radius:var(--md-shape-corner-xs);border:1px solid var(--md-sys-color-outline-variant);padding:8px 10px;background:var(--md-sys-color-surface-container-high);color:var(--md-sys-color-on-surface);font-size:12.5px;font-family:inherit;resize:vertical;min-height:60px;">${esc(saved)}</textarea>
        <button class="btn-m3-primary" style="width:100%" data-drawer-hand="${esc(task.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">send</span>
          <span>待機キューに渡す</span>
        </button>
      </div>
    `;
  const textarea = el.querySelector('#drawer-instruction');
  el.querySelector('[data-drawer-hand]').addEventListener('click', () =>
    hand(task.id, textarea.value.trim()));
  textarea.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing) {
      e.preventDefault();
      hand(task.id, textarea.value.trim());
    }
  });
}


