/* In 「すべて」 a card says which board it came from; on a board of its own that is known. */
function originChip(item) {
  // On the repository's board a parent-task hub's card says whose it is, by the parent's key.
  if (item?.ownerHub && !scopeAll()) {
    const h = (state.hubs || []).find(x => x.slug === item.ownerHub.slug);
    const name = item.ownerHub.key || (h ? hubShortName(h) : item.ownerHub.slug);
    const title = (h && hubTitle(h)) || (h ? hubLabel(h) : name);
    return `<span class="origin-chip" title="${esc(title)}"><span class="material-symbols-outlined" aria-hidden="true">account_tree</span><span>${esc(name)}</span></span>`;
  }
  if (!scopeAll() || !item?._slug) return '';
  const b = boards.find(x => x.slug === item._slug);
  const name = b ? (b.hub || repoNameOf(b)) : item._slug;
  const title = b ? boardName(b) + (b.hub ? ` (${b.nwo})` : '') : item._slug;
  return `<span class="origin-chip" title="${esc(title)}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(name)}</span></span>`;
}

/* What a card that waits on the person says it waits for, by the kind of the gate that is open. */
const WAIT_OF_KIND = {
  dispatch: '着手確認',
  issue: '起票の確認',
  plan: '計画',
  diff: '差分',
  verify: '動作確認',
  result: '報告',
  question: '回答',
  relay: '指摘の転送',
};

/* The same for a PR on the person's turn with no gate open, by whose turn it is. A turn not listed is the plain review. */
const PR_TURN_WAIT = {
  changes: '修正の依頼への対応',
  merge: 'マージ',
  'ci-failed': 'CI の失敗への対応',
  closed: '閉じた PR の判断',
};

/* The words after 「…を待っています」 on a card whose ball is the person's (`hcol` is its column, from `humanColOf`). A card of a
   parent-task hub on the repository's board has no gate or PR turn of its own to read, only the column. */
function waitWordOf(task, hcol) {
  if (task.ownerHub) return humanLabel(hcol);
  const gate = openGate(task);
  if (gate) return WAIT_OF_KIND[gate.kind] || humanLabel(hcol);
  if (task.waitsOnPerson) return PR_TURN_WAIT[task.prTurn] || 'PR のレビュー';
  return humanLabel(hcol);
}

function agentCard(task) {
  const el = document.createElement('div');
  // A card of a parent-task hub on the repository's board: read from that hub's records, so
  // what it does goes to that hub's board, and without the resident server it only shows.
  const owner = task.ownerHub;
  const hcol = owner ? owner.humanCol : humanColOf(task);
  const compact = task.status === 'backlog' || task.status === 'done';
  const stuck = stuckOf(task);
  // A worker waiting on a permission prompt asks a person too, though no gate is open. Not
  // known in 「すべて」, which has no sessions of its own (sessionOfTask is then not a session).
  const asking = ['dispatched', 'pr'].includes(task.status) && !hcol ? sessionOfTask(task) : null;
  const asks = !!asking && sessionState(asking) === 'permission';
  el.className = 'card' + (hcol || asks ? ' waiting' : '') + (stuck && !hcol ? ' stuck' : '') + (compact ? ' compact' : '') + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `agent-${task.id}`;
  el.dataset.id = task.id;
  if (task._slug) el.dataset.slug = task._slug;
  if (owner) {
    el.dataset.slug = owner.slug;
    el.dataset.owner = '1';
  }

  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;
  // A gate is open and the worker went on past it: the card shows what the worker does now, and the gate as a second mark.
  const went = !!hcol && !owner && taskPassedGate(task);
  const wentSession = went ? sessionOfTask(task) : null;
  const wentStatus = went ? ledgerState(wentSession, state) : '';

  let h = '';

  // 1. Header row
  const doneLabel = DONE_WHEN[task.doneWhen] || task.doneWhen;
  const donePillClass = task.doneWhen === 'report-only' ? 'pill-purple'
    : task.doneWhen === 'verify' ? 'pill-warn'
    : task.doneWhen === 'review' ? 'pill-blue'
    : 'pill-neutral';

  const issueUrl = httpUrl(task.issueUrl);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : null;

  h += `
    <div class="card-header-row">
      ${issueNumber ? '' : `<span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>`}
      ${ghChipsHtml(task)}
      ${doneLabel ? `<span class="m3-pill ${donePillClass}">${esc(doneLabel)}</span>` : ''}
      ${task.status === 'queued' && task.order != null && !owner ? `<span class="m3-pill pill-neutral" title="キューの優先順"><span class="material-symbols-outlined" style="font-size:12px;">swap_vert</span>キュー ${task.order}</span>` : ''}
      ${task.status === 'backlog' ? '<span class="m3-pill pill-neutral" title="まだキューに渡していません">Backlog</span>' : ''}
    </div>
  `;

  // 2. Title
  h += originChip(task);
  h += `<div class="title">${esc(task.title)}${titlePendingPill(task)}</div>`;

  // 2.5 What the card waits for, on one line: the person's turn, or a permission prompt (which is the worker's own question)
  if (hcol) {
    h += `<div class="card-wait-line">${esc(waitWordOf(task, hcol))}を待っています</div>`;
  } else if (asks) {
    h += `<div class="card-wait-line err">${esc(permissionLabel(asking))}</div>`;
  }

  // 3. Worker Status (or Jules)
  if (live && task.jules) {
    h += julesLine(task);
  } else if (worker && worker.phase) {
    const mins = phaseMinutes(worker);
    const stopped = !worker.present;
    // A worker gone once its PR is open has finished rather than stopped (see stuckOf).
    const alarm = stopped && !hcol && task.status !== 'pr';
    h += `
      <div class="card-worker-status">
        ${went ? (wentStatus === 'working' ? '<span class="pulse-dot"></span>'
                 : `<span class="material-symbols-outlined" aria-hidden="true" style="font-size:14px;color:var(--md-sys-color-${wentStatus === 'failed' ? 'error' : 'warning'});">${wentStatus === 'failed' ? 'error' : 'front_hand'}</span>`)
               : hcol ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-warning);">pause_circle</span>'
               : asks ? '<span class="material-symbols-outlined" aria-hidden="true" style="font-size:14px;color:var(--md-sys-color-error);">front_hand</span>'
               : alarm ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-error);">pause</span>'
               : stopped ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-outline);">pause</span>'
               : '<span class="pulse-dot"></span>'}
        <span style="font-weight:700;${alarm ? 'color:var(--md-sys-color-error);' : ''}">${esc(PHASE_LABEL[worker.phase] || worker.phase)}${alarm ? ' (停止)' : ''}</span>
        ${went && wentStatus === 'permission' ? `<span style="font-weight:700;color:var(--md-sys-color-warning);">${permissionLabel(wentSession)}</span>` : ''}
        ${went && wentStatus === 'failed' ? '<span style="font-weight:700;color:var(--md-sys-color-error);">エラー</span>' : ''}
        ${went ? '<span style="font-weight:700;color:var(--md-sys-color-warning);">判定待ち（答えないまま進行中）</span>' : ''}
        ${mins != null ? `<span class="ago" style="margin-left:auto;color:var(--md-sys-color-outline);font-size:11px;">${minutesLabel(mins)}</span>` : ''}
      </div>
      ${asks && (asking.agentSession?.request || asking.prompt) ? `<div style="font-size:11px;color:var(--md-sys-color-outline);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title="${esc(asking.agentSession?.request ? (isQuestion(asking) ? requestText(asking) : asking.agentSession.request) : workPromptText(asking.prompt, asking))}">${esc(asking.agentSession?.request ? (isQuestion(asking) ? requestText(asking) : asking.agentSession.request) : workPromptText(asking.prompt, asking))}</div>` : ''}
    `;
  }

  // 4. Metadata pills
  const metaBadges = [];
  if (task.executor === 'jules' && !task.jules) {
    metaBadges.push(`<span class="m3-pill pill-purple" title="計画の承認後に Jules へ渡す"><span class="material-symbols-outlined" style="font-size:12px;">smart_toy</span>Jules</span>`);
  }
  if (!task.autoStart && task.status !== 'done') {
    metaBadges.push(`<span class="m3-pill pill-warn" title="着手前に確認が必要"><span class="material-symbols-outlined" style="font-size:12px;">lock</span>要着手確認</span>`);
  }
  // Whose turn the PR is on (the person's, somebody else's, or the bots'), where it has a word for it.
  const turnPill = prTurnPill(task);
  if (turnPill) metaBadges.push(turnPill);
  if (!httpUrl(task.pr) && live && worker?.phase === 'pr') {
    metaBadges.push('<span class="m3-pill pill-neutral"><span class="material-symbols-outlined" style="font-size:12px;">hourglass_top</span>PR 作成中</span>');
  }
  if (metaBadges.length) {
    h += `<div class="pills" style="display:flex;flex-wrap:wrap;gap:4px;">${metaBadges.join('')}</div>`;
  }

  // 5. Chips (diff / verify records)
  // A record chip opens its record, which only this board's own tasks or a switch to the owning board can show.
  const chips = owner && !multiBoard ? [] : chipsOf(task);
  if (chips.length) {
    h += `<div class="chips" style="display:flex;flex-wrap:wrap;gap:4px;">${chips.map(c => {
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

  // 6. Note
  if (task.note) {
    if (task.status === 'done') {
      h += `<div style="font-size:11.5px;color:var(--md-sys-color-outline);overflow-wrap:anywhere;">${esc(task.note)}</div>`;
    } else {
      h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11.5px;color:var(--md-sys-color-error);font-weight:600;"><span class="material-symbols-outlined" style="font-size:14px;flex-shrink:0;">warning</span><span>${esc(task.note)}</span></div>`;
    }
  }

  // 6.5 Instruction
  if (task.instruction && task.status !== 'backlog') {
    h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11px;color:var(--md-sys-color-primary);background:var(--md-sys-color-surface-container-high);padding:4px 8px;border-radius:var(--md-shape-corner-xs);margin-top:2px;">
      <span class="material-symbols-outlined" style="font-size:13px;flex-shrink:0;margin-top:1px;">forward_to_inbox</span>
      <span style="overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;" title="${esc(task.instruction)}">${esc(task.instruction)}</span>
    </div>`;
  }

  // 7. Stuck warning (only if NOT waiting on human)
  if (stuck && !hcol) {
    h += `<div style="display:flex;align-items:center;gap:4px;font-size:11px;color:var(--md-sys-color-error);font-weight:700;"><span class="material-symbols-outlined" style="font-size:14px;">timer</span><span>${esc(stuck)}</span></div>`;
  }

  // 8. Done completed time
  if (task.status === 'done' && task.updatedAt) {
    h += `<div style="font-size:11px;color:var(--md-sys-color-outline);">${ago(task.updatedAt)}に完了</div>`;
  }

  // 9. Footer row: slug & buttons
  const worktreeSlug = baseName(task.worktree);
  const branchSlug = task.branch || '';
  const hasSlug = worktreeSlug || branchSlug || (!issueNumber && task.id);
  const hasButtons = task.worktree || task.status === 'backlog';

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
          ${task.worktree && (!owner || multiBoard) ? `<button type="button" class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">code</span><span>IDE</span></button>` : ''}
          ${task.status === 'backlog' && (!owner || multiBoard) ? `<button type="button" class="m3-icon-button" style="color:var(--md-sys-color-primary);" title="待ちキューへ渡す" data-hand="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span><span>渡す</span></button>` : ''}
        </div>
      </div>
    `;
  }

  el.innerHTML = h;
  if (owner && !multiBoard) {
    el.style.cursor = 'default';
    return el;
  }
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('.card-issue-link') || e.target.closest('a')) return;
    const slug = owner ? owner.slug : task._slug;
    // A board served alone has no terminal beside it: its card opens the task panel.
    if (!multiBoard) return onBoard(slug, () => openTaskPanel(task.id));
    // A card with a worker, or with a question waiting, opens its terminal beside the board; the others have none, so their detail opens.
    if (live || hcol) return onBoard(slug, () => openTaskPanel(task.id, 'term'));
    onBoard(slug, () => openTaskDetail((state.tasks || []).find(t => t.id === task.id) || task, el));
  };
  pressableCard(el, task.title || task.id);
  return el;
}

/* A session started with no task: it has no record to show gates or a phase for, so the card
   says what it is and offers what it can become (#178). `s` is its session, which the 「すべて」
   view does not always carry. */
/* A card that opens something when pressed is reached and pressed from the keyboard too: pressing it is the only way to its terminal.
   Enter or Space on the card itself presses it; on a control inside it, that control's own. */
function pressableCard(el, label) {
  el.tabIndex = 0;
  el.setAttribute('role', 'button');
  el.setAttribute('aria-label', label);
  el.addEventListener('keydown', e => {
    if (e.target !== el || e.isComposing || (e.key !== 'Enter' && e.key !== ' ')) return;
    e.preventDefault();
    el.click();
  });
}

function sessionCard(w) {
  const s = (state.sessions || []).find(x => x.kind === 'worker' && x.worktree === w.worktree && (!w._slug || x._slug === w._slug)) || null;
  const el = document.createElement('div');
  el.className = 'card session-card';
  el.dataset.worktree = w.worktree;
  if (w._slug) el.dataset.slug = w._slug;

  const missing = !!w.task && !(state.tasks || []).some(t => t.id === w.task && (!w._slug || t._slug === w._slug));
  const what = missing ? 'タスク ID はこのボードに見つかりません' : 'タスクのレコードがないセッション';
  const folder = baseName(w.worktree) || w.name || '';
  const name = s ? sessionKey(s) : folder;
  const icon = n => `<span class="material-symbols-outlined" style="font-size:14px;">${n}</span>`;

  let h = `<div class="card-header-row"><span class="m3-pill pill-neutral" title="${esc(what)}">${icon('link_off')}<span>${missing ? 'タスクが見つからない' : 'タスクなし'}</span></span></div>
    <div class="title" title="${esc(w.branch || '')}">${esc(name)}</div>`;

  const st = s ? sessionState(s) : null;
  const last = s ? lastOutputText(s) : null;
  h += `<div class="card-worker-status">
    ${st ? `<span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(ROW_LABEL[st] || STATE_LABEL[st] || st)}</span>`
         : `<span class="m3-pill ${w.present ? 'pill-good' : 'pill-neutral'}">${w.present ? '稼働' : '停止'}</span>`}
    ${last ? `<span>最後の出力 ${esc(last)}</span>` : ''}
    ${s && agentText(s) ? `<span class="card-worker-cmd">${esc(agentText(s))}</span>` : ''}
  </div>`;

  // Linking writes the task and tells the worker, which a session that is not running cannot read.
  const off = s && !s.present ? ' disabled title="セッションが動いていないため、タスクにも既存のタスクへの紐づけもできません"' : '';
  const link = (mode, label) => `<button type="button" class="m3-icon-button" data-sess-link="${mode}" data-sref="${esc(sessionRef(s))}"${off}><span>${label}</span></button>`;
  h += `<div class="card-footer-row">
    <div class="card-meta-slug" title="${esc(w.worktree || '')}"><span class="material-symbols-outlined" style="font-size:13px;">folder_open</span><span>${esc(folder)}</span></div>
    <div class="card-button-row">
      ${s && !missing ? link('new', 'タスクにする…') + link('existing', '紐づける…') : ''}
      ${w.worktree ? `<button type="button" class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(w.worktree)}">${icon('code')}<span>IDE</span></button>` : ''}
    </div>
  </div>`;

  el.innerHTML = h;
  // Without a terminal there is no session to open.
  const opens = !!s && boardTerminalReady(s);
  // 「すべて」 carries no sessions: the board's own page does, with the buttons.
  const lands = !s && !!w._slug;
  if (!opens && !lands) el.style.cursor = 'default';
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('a')) return;
    if (opens) onBoard(w._slug, () => openSessionRef(sessionRef(s)));
    else if (lands) onBoard(w._slug, () => {});
  };
  if (opens || lands) pressableCard(el, name);
  return el;
}
