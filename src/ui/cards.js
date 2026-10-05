/* In 「すべて」 a card says which board it came from; on a board of its own that is known. */
function originChip(item) {
  if (!scopeAll() || !item?._slug) return '';
  const b = boards.find(x => x.slug === item._slug);
  const name = b ? (b.hub || repoNameOf(b)) : item._slug;
  const title = b ? boardName(b) + (b.hub ? ` (${b.nwo})` : '') : item._slug;
  return `<span class="origin-chip" title="${esc(title)}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(name)}</span></span>`;
}

function humanActions(task, col, gate) {
  const open = openReplies[task.id];
  if (open) {
    const ph = open === 'answer' ? '回答を入力してください...'
             : open === 'ask' ? '追加で聞きたい内容を入力してください...'
             : open === 'changes' ? '修正指示や指摘を入力してください...'
             : '理由を入力してください...';
    const sendLabel = !gate && open === 'changes' ? '指摘をメモ' : actLabel(open, gate?.kind);
    return `
      <div class="hcard-reply">
        <textarea data-reply="${esc(task.id)}" placeholder="${esc(ph)}"></textarea>
        <div class="hcard-actions">
          <button type="button" class="btn-m3-primary" data-act="send-${open}" data-id="${esc(task.id)}">
            <span class="material-symbols-outlined">send</span>
            <span>${esc(sendLabel)}</span>
          </button>
          <button type="button" class="btn-m3-text" data-act="cancel" data-id="${esc(task.id)}">やめる</button>
        </div>
      </div>
    `;
  }
  let buttons = '';
  if (col === 'dispatch') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="start" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">play_arrow</span><span>${esc(actLabel('start', gate?.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="shelve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">undo</span><span>${gate ? esc(actLabel('shelve', gate.kind)) : 'Backlog に戻す'}</span>
      </button>
    `;
  } else if (col === 'plan') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>${esc(actLabel('approve', gate?.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">reply</span><span>${esc(actLabel('reject', gate?.kind))}</span>
      </button>
    `;
  } else if (col === 'diff') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>${esc(actLabel('approve', gate?.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">reply</span><span>${esc(actLabel('reject', gate?.kind))}</span>
      </button>
    `;
  } else if (col === 'verify') {
    const isResult = gate?.kind === 'result';
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>${esc(actLabel('approve', gate?.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="${isResult ? 'ask' : 'reject'}" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">${isResult ? 'help' : 'reply'}</span><span>${esc(actLabel(isResult ? 'ask' : 'reject', gate?.kind))}</span>
      </button>
    `;
  } else if (col === 'prreview') {
    buttons = `
      ${scopeAll() ? '' : `<button type="button" class="btn-m3-primary" data-act="refresh-prs" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">sync</span><span>PR確認</span>
      </button>`}
      <button type="button" class="${scopeAll() ? 'btn-m3-tonal' : 'btn-m3-text'}" data-act="changes" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">rate_review</span><span>指摘をメモ</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">chat</span><span>${esc(actLabel('answer', gate?.kind))}</span>
      </button>
      ${readySessionOfTask(task) ? `<button type="button" class="btn-m3-tonal" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}">
        <span class="material-symbols-outlined">terminal</span><span>ターミナルで答える</span>
      </button>` : ''}
    `;
  }
  if (gate && gate.choices && gate.choices.length) {
    buttons += gate.choices.map(c => `
      <button type="button" class="btn-m3-tonal" data-act="choice" data-choice="${esc(c.id)}" data-id="${esc(task.id)}">
        <span>${esc(c.label || c.id)}</span>
      </button>
    `).join('');
  }
  return `<div class="hcard-actions">${buttons}</div>`;
}

function humanGateActions(gate, col) {
  const open = openReplies[gateRef(gate)];
  if (open) {
    const ph = open === 'answer' ? '回答を入力してください...'
             : open === 'ask' ? '追加で聞きたい内容を入力してください...'
             : open === 'changes' ? '修正指示や指摘を入力してください...'
             : '理由を入力してください...';
    const sendLabel = actLabel(open, gate.kind);
    return `
      <div class="hcard-reply">
        <textarea data-reply="${esc(gateRef(gate))}" placeholder="${esc(ph)}"></textarea>
        <div class="hcard-actions">
          <button type="button" class="btn-m3-primary" data-act="send-${open}" data-id="${esc(gateRef(gate))}">
            <span class="material-symbols-outlined">send</span>
            <span>${esc(sendLabel)}</span>
          </button>
          <button type="button" class="btn-m3-text" data-act="cancel" data-id="${esc(gateRef(gate))}">やめる</button>
        </div>
      </div>
    `;
  }
  let buttons = '';
  if (col === 'dispatch') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">play_arrow</span><span>${esc(actLabel('approve', gate.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">undo</span><span>${esc(actLabel('reject', gate.kind))}</span>
      </button>
    `;
  } else if (col === 'plan' || col === 'diff') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">check</span><span>${esc(actLabel('approve', gate.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">reply</span><span>${esc(actLabel('reject', gate.kind))}</span>
      </button>
    `;
  } else if (col === 'verify') {
    const isResult = gate.kind === 'result';
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">check</span><span>${esc(actLabel('approve', gate.kind))}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="${isResult ? 'ask' : 'reject'}" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">${isResult ? 'help' : 'reply'}</span><span>${esc(actLabel(isResult ? 'ask' : 'reject', gate.kind))}</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">chat</span><span>${esc(actLabel('answer', gate.kind))}</span>
      </button>
    `;
  }
  if (gate.choices && gate.choices.length) {
    buttons += gate.choices.map(c => `
      <button type="button" class="btn-m3-tonal" data-act="choice" data-choice="${esc(c.id)}" data-id="${esc(gateRef(gate))}">
        <span>${esc(c.label || c.id)}</span>
      </button>
    `).join('');
  }
  return `<div class="hcard-actions">${buttons}</div>`;
}

/* What a person is asked to do about a PR, by whose turn it is. A turn not listed is today's
   plain 「GitHub でレビューしてください」. */
const PR_TURN_WHY = {
  changes: 'レビューで修正の依頼がありました。worker に直させるか自分で対応してください',
  merge: '承認されました。マージしてください',
  'ci-failed': 'CI が失敗しています',
  closed: 'PR がマージされずに閉じられました。別の PR で続けるか、取り消すかを決めてください',
};

function humanCard(task, col) {
  const el = document.createElement('div');
  el.className = 'card hcard' + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `human-${task.id}`;
  el.dataset.id = task.id;
  if (task._slug) el.dataset.slug = task._slug;

  const gate = openGate(task);
  const mins = waitingMinutes(task);
  const issueUrl = httpUrl(task.issueUrl);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : null;

  let why = '';
  if (gate) {
    if (gate.problem) why = gate.problem;
    else if (gate.why) why = gate.why;
    else if (stopWhy(gate).length) why = stopWhy(gate).join(' / ');
    else why = gate.title;
  } else if (col === 'prreview') {
    why = PR_TURN_WHY[task.prTurn] || 'worker は PR を出して待っています。GitHub でレビューしてください';
  }

  const w = workerOf(task);
  const phaseStr = task.status === 'queued' ? '着手前'
    : (w?.phase ? `${PHASE_LABEL[w.phase] || w.phase}で停止中` : `${task.status}で停止中`);

  el.innerHTML = `
    <div class="card-header-row">
      ${issueNumber ? '' : `<span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>`}
      ${ghChipsHtml(task)}
      ${col === 'prreview' ? prTurnPill(task) : ''}
      <span class="wait-time ${waitTone(mins)}" title="待たせている時間">
        <span class="material-symbols-outlined">schedule</span>
        <span>${minutesLabel(mins)}待ち</span>
      </span>
    </div>
    ${originChip(task)}
    <div class="title">${esc(task.title)}${titlePendingPill(task)}</div>
    ${col === 'question' ? `<div class="question-box">${esc(why)}</div>` : (why ? `<div class="why">${esc(why)}</div>` : '')}
    ${humanActions(task, col, gate)}
    <div class="hcard-foot">
      <span>${esc(phaseStr)}</span>
      <span class="hcard-foot-links">
        ${readySessionOfTask(task) ? `<button type="button" class="agent-back" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}">
          <span class="material-symbols-outlined">terminal</span>
          <span>ターミナル</span>
        </button>` : ''}
        <button type="button" class="agent-back" data-jump-agent="${esc(task.id)}" title="エージェントのボードでこのカードを見る">
          <span class="material-symbols-outlined">smart_toy</span>
          <span>エージェントで見る</span>
        </button>
      </span>
    </div>
  `;

  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('a') || e.target.closest('textarea') || e.target.closest('input')) return;
    onBoard(task._slug, () => openTaskPanel(task.id));
  };
  return el;
}

function humanGateCard(gate, col) {
  const el = document.createElement('div');
  el.className = 'card hcard';
  el.id = `human-${gate.id}`;
  el.dataset.id = gateRef(gate);
  if (gate._slug) el.dataset.slug = gate._slug;

  const mins = stampSecs(gate.openedAt) ? Math.max(0, Math.floor((Date.now() - stampSecs(gate.openedAt) * 1000) / 60000)) : 0;
  const wtName = gate.worktree ? gate.worktree.split('/').pop() : gate.id;
  const why = gate.problem || gate.why || stopWhy(gate).join(' / ') || gate.title;

  el.innerHTML = `
    <div class="card-header-row">
      <span class="card-task-id" title="${esc(gate.id)}">${esc(wtName)}</span>
      <span class="wait-time ${waitTone(mins)}" title="待たせている時間">
        <span class="material-symbols-outlined">schedule</span>
        <span>${minutesLabel(mins)}待ち</span>
      </span>
    </div>
    ${originChip(gate)}
    <div class="title">${esc(gate.title)}</div>
    ${col === 'question' ? `<div class="question-box">${esc(why)}</div>` : `<div class="why">${esc(why)}</div>`}
    ${humanGateActions(gate, col)}
    <div class="hcard-foot">
      <span>${esc(wtName)}</span>
      <button type="button" class="btn-m3-text" style="padding:0;font-size:11px;" data-gate="${esc(gate.id)}">詳細判定画面 →</button>
    </div>
  `;

  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('a') || e.target.closest('textarea') || e.target.closest('input')) return;
    onBoard(gate._slug, () => judgeGate(gate.id));
  };
  return el;
}

function agentCard(task) {
  const el = document.createElement('div');
  const hcol = humanColOf(task);
  const compact = task.status === 'backlog' || task.status === 'done';
  const stuck = stuckOf(task);
  el.className = 'card' + (hcol ? ' waiting' : '') + (stuck && !hcol ? ' stuck' : '') + (compact ? ' compact' : '') + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `agent-${task.id}`;
  el.dataset.id = task.id;
  if (task._slug) el.dataset.slug = task._slug;

  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;

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
      ${task.status === 'queued' && task.order != null ? `<span class="m3-pill pill-neutral" title="キューの優先順"><span class="material-symbols-outlined" style="font-size:12px;">swap_vert</span>${task.order}</span>` : ''}
    </div>
  `;

  // 2. Title
  h += originChip(task);
  h += `<div class="title">${esc(task.title)}${titlePendingPill(task)}</div>`;

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
        ${hcol ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-warning);">pause_circle</span>'
               : alarm ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-error);">pause</span>'
               : stopped ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-outline);">pause</span>'
               : '<span class="pulse-dot"></span>'}
        <span style="font-weight:700;${alarm ? 'color:var(--md-sys-color-error);' : ''}">${esc(PHASE_LABEL[worker.phase] || worker.phase)}${alarm ? ' (停止)' : ''}</span>
        ${mins != null ? `<span class="ago" style="margin-left:auto;color:var(--md-sys-color-outline);font-size:11px;">${minutesLabel(mins)}</span>` : ''}
      </div>
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
  // A PR that waits on somebody else, or on the bots, is off the person's board but is not the
  // worker's to act on either; say whose it is.
  if (!hcol && ['other-reviewer', 'checks'].includes(task.prTurn)) {
    metaBadges.push(prTurnPill(task));
  }
  if (!httpUrl(task.pr) && live && worker?.phase === 'pr') {
    metaBadges.push('<span class="m3-pill pill-neutral"><span class="material-symbols-outlined" style="font-size:12px;">hourglass_top</span>PR 作成中</span>');
  }
  if (metaBadges.length) {
    h += `<div class="pills" style="display:flex;flex-wrap:wrap;gap:4px;">${metaBadges.join('')}</div>`;
  }

  // 5. Chips (diff / verify records)
  const chips = chipsOf(task);
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
  const worktreeSlug = task.worktree ? task.worktree.split('/').pop() : '';
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
          ${readySessionOfTask(task) ? `<button type="button" class="m3-icon-button" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">terminal</span><span>ターミナル</span></button>` : ''}
          ${task.worktree ? `<button type="button" class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">code</span><span>IDE</span></button>` : ''}
          ${task.status === 'backlog' ? `<button type="button" class="m3-icon-button" style="color:var(--md-sys-color-primary);" title="待ちキューへ渡す" data-hand="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span><span>渡す</span></button>` : ''}
        </div>
      </div>
    `;
  }

  // 10. Wait link if waiting on human
  if (hcol) {
    const mins = waitingMinutes(task);
    h += `
      <button type="button" class="wait-link" data-jump-human="${esc(task.id)}" title="人のボードでこのカードを開く">
        <div class="wait-link-row">
          <span class="wait-link-badge">
            <span class="material-symbols-outlined">person_alert</span>
            <span>人の確認待ち</span>
          </span>
          <span class="go">${minutesLabel(mins)}<span class="material-symbols-outlined">arrow_outward</span></span>
        </div>
        <div class="wait-link-target">${esc(humanLabel(hcol))}</div>
      </button>
    `;
  }

  el.innerHTML = h;
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('.card-issue-link') || e.target.closest('a')) return;
    onBoard(task._slug, () => openTaskPanel(task.id));
  };
  return el;
}

/* A session started with no task: it has no record to show gates or a phase for, so the card
   says what it is and offers what it can become (#178). `s` is its session, which the 「すべて」
   view does not always carry. */
function sessionCard(w) {
  const s = (state.sessions || []).find(x => x.kind === 'worker' && x.worktree === w.worktree && (!w._slug || x._slug === w._slug)) || null;
  const el = document.createElement('div');
  el.className = 'card session-card';
  el.dataset.worktree = w.worktree;
  if (w._slug) el.dataset.slug = w._slug;

  const missing = !!w.task && !(state.tasks || []).some(t => t.id === w.task && (!w._slug || t._slug === w._slug));
  const what = missing ? 'タスク ID はこのボードに見つかりません' : 'タスクのレコードがないセッション';
  const folder = (w.worktree || '').split('/').filter(Boolean).pop() || w.name || '';
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
  </div>`;

  // Linking writes the task and tells the worker, which a session that is not running cannot read.
  const off = s && !s.present ? ' disabled title="セッションが動いていないため、タスクにも既存のタスクへの紐づけもできません"' : '';
  const link = (mode, label) => `<button type="button" class="m3-icon-button" data-sess-link="${mode}" data-sref="${esc(sessionRef(s))}"${off}><span>${label}</span></button>`;
  h += `<div class="card-footer-row">
    <div class="card-meta-slug" title="${esc(w.worktree || '')}"><span class="material-symbols-outlined" style="font-size:13px;">folder_open</span><span>${esc(folder)}</span></div>
    <div class="card-button-row">
      ${s && boardTerminalReady(s) ? `<button type="button" class="m3-icon-button" title="内蔵ターミナルをセッションの画面で開く" data-sess-open="${esc(sessionRef(s))}">${icon('terminal')}<span>ターミナル</span></button>` : ''}
      ${s && !missing ? link('new', 'タスクにする…') + link('existing', '紐づける…') : ''}
      ${w.worktree ? `<button type="button" class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(w.worktree)}">${icon('code')}<span>IDE</span></button>` : ''}
    </div>
  </div>`;

  el.innerHTML = h;
  // Without a terminal the Sessions view is not available and would bounce back.
  const opens = !!s && boardTerminalReady(s);
  // 「すべて」 carries no sessions: the board's own page does, with the buttons.
  const lands = !s && !!w._slug;
  if (!opens && !lands) el.style.cursor = 'default';
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('a')) return;
    if (opens) onBoard(w._slug, () => openSessionRef(sessionRef(s)));
    else if (lands) onBoard(w._slug, () => {});
  };
  return el;
}
