/* ── The task detail: the pure half (src/ui/task-detail-view.js opens, draws and wires it) ────────────────────────────────────────
   One task as one dialog with a table of contents: 概要 (the request, the goal, the plan, the questions asked and how they were
   answered, what it does now), 変更 (the diff and the checks), 経過 (the timeline) and 詳細 (where it lives and how it was set up).
   These functions take the task and what surrounds it and return markup, touching neither the page, `state`, `nav` nor the network, so
   `src/ui/tests/task-detail.test.js` runs them under `node --test`. Everything that comes from the page arrives as an argument.
   Loaded after question-card.js (`qcIcon`) and after util.js (`esc`, `md`, `when`, `ago`, `kindOf`, `DECISION`, `decisionLabel`,
   `handoffText`), decide.js (`gateRef`, the review and check cards), gate-evidence.js (`gateFocusLine`), board.js (`DONE_WHEN`,
   `STOP_AT`) and task-view.js (`timelineHtml`, `diffCardHtml`, `filesTableHtml`, `issueNumberOf`, `prRefNumber`): they are called, not
   read, when the page loads. Nothing here answers a gate: questions are answered over the terminal and in the TODO. */

const TD_ICON = qcIcon;

/* The frame of a dialog with a table of contents. It knows nothing about tasks, so another dialog can use it. `groups` are
   [{ key, icon, label, html }], `html` being markup the caller has built; `sub` is the strings of the line over the title, joined by
   ` · `; `now` is one line under the title, left out when empty; `idPrefix` keeps the ids of two such dialogs apart. Returns the head
   and the body for `dialogStack.push`. */
function tocDialogHtml({ sub = [], title = '', now = '', groups = [], idPrefix = 'td' }) {
  const line = (sub || []).filter(Boolean).join(' · ');
  const head = `${line ? `<div class="qd-sub">${esc(line)}</div>` : ''}<h2 class="qd-title">${esc(title)}</h2>`
    + (now ? `<p class="td-now">${TD_ICON('schedule')}<span>${esc(now)}</span></p>` : '');
  const toc = groups.map((g, i) =>
    `<button type="button" data-td-go="${esc(g.key)}"${i === 0 ? ' aria-current="location"' : ''}>${TD_ICON(g.icon)}<span>${esc(g.label)}</span></button>`).join('');
  const sections = groups.map(g => {
    const id = `${idPrefix}-title-${g.key}`;
    return `<section class="td-group" data-td-group="${esc(g.key)}" aria-labelledby="${esc(id)}" tabindex="-1">`
      + `<h3 class="td-group-title" id="${esc(id)}">${TD_ICON(g.icon)}<span>${esc(g.label)}</span></h3>${g.html}</section>`;
  }).join('');
  return { head, body: `<div class="td-split"><nav class="td-toc" aria-label="目次">${toc}</nav><div class="td-content">${sections}</div></div>` };
}

/* The index of the group the table of contents marks: the last whose top is at or above the top of the view (plus `offset`, so a
   heading just under the edge counts). At the very end of the scroll it is the last, for a short last group never reaches the top. */
function tocActiveKey(tops, scrollTop, clientHeight, scrollHeight, offset = 40) {
  if (!tops.length) return 0;
  if (scrollTop > 0 && scrollTop + clientHeight >= scrollHeight - 2) return tops.length - 1;
  let at = 0;
  tops.forEach((top, i) => { if (top <= scrollTop + offset) at = i; });
  return at;
}

/* ── the head ── */

/* The line over the title: the repository, the Issue, the PR and the phase; what the task does not have is left out. */
function taskDetailSub(task, { repo = '', phase = '' } = {}) {
  const issue = task.issueUrl ? issueNumberOf(task.issueUrl) : '';
  const pr = task.pr ? prRefNumber(task.pr) : '';
  return [repo, issue ? `Issue #${issue}` : '', pr ? `PR #${pr}` : '', phase].filter(Boolean);
}

/* What the task is waiting for or doing, in one line. `waiting` are the gates asking a person, oldest first; `worker` the task's worker
   when there is one; `phaseMins` how long it has been in its phase; `prNote` what the PR waits on. */
function taskDetailNowText(task, { waiting = [], worker = null, phaseMins = null, prNote = '' } = {}) {
  if (waiting.length === 1) {
    const g = waiting[0];
    return g.kind === 'dispatch' ? '着手確認を待っています' : `「${kindOf(g.kind)[0]}」を待っています`;
  }
  if (waiting.length > 1) return `確認待ちが ${waiting.length} 件あります（${waiting.map(g => kindOf(g.kind)[0]).join('・')}）`;
  if (worker?.present && worker.phase) {
    return `worker は${PHASE_LABEL[worker.phase] || worker.phase}${phaseMins != null ? `（${minutesLabel(phaseMins)}前から）` : ''}`;
  }
  if (task.status === 'done') return '完了しました';
  if (task.status === 'cancelled') return '取り消しました';
  if (task.status === 'pr' && prNote) return prNote;
  if (task.status === 'backlog' || task.status === 'queued') return '着手前です';
  return '';
}

/* ── 概要 ── */

const tdRow = (icon, label, inner) => `<div class="td-row"><h4>${TD_ICON(icon)}<span>${esc(label)}</span></h4><div>${inner}</div></div>`;
const tdMuted = text => `<div class="tp-muted">${esc(text)}</div>`;

/* How an answered gate was answered, as text: the choice picked, what the points of a read-through sent, or the decision. */
function taskDetailPickText(g) {
  if (g.handoff) return handoffText(g.handoff);
  if (g.choice) return (g.choices || []).find(c => c.id === g.choice)?.label || g.choice;
  return DECISION_LABEL[g.decision] ? decisionLabel(g.decision, g.kind) : DECISION[g.decision] || g.decision;
}

/* The other ways it could have been answered: the choices not picked, or for a gate with none, the other options. */
function taskDetailOthers(g) {
  if ((g.choices || []).length) return g.choices.filter(c => c.id !== g.choice).map(c => c.label);
  return (g.options || []).filter(o => o !== g.decision && o !== 'close').map(o => decisionLabel(o, g.kind));
}

/* The question of a gate in a line. */
const tdQuestionOf = g => gateFocusLine(g.focus) || g.title || '';

function tdKindChip(kind) {
  const [label, color] = kindOf(kind);
  return `<span class="qc-kind"><i style="background:${esc(color)}"></i>${esc(label)}</span>`;
}

/* One question that was asked and answered. */
function tdAnswerHtml(g) {
  const others = taskDetailOthers(g);
  return '<div class="td-qa">'
    + `<div class="td-qa-head"><span class="td-qa-at" title="${esc(ago(g.answeredAt))}">${esc(when(g.answeredAt))}</span>${tdKindChip(g.kind)}<span class="td-qa-q">${esc(tdQuestionOf(g))}</span></div>`
    + '<div class="td-qa-body">'
    + `<div class="td-qa-pick">${TD_ICON('check')}<span>${esc(taskDetailPickText(g))}</span></div>`
    + (others.length ? `<div class="td-qa-others">ほかの選択肢: ${esc(others.join(' / '))}</div>` : '')
    + (g.comment ? (g.handoff ? handoffDetailsHtml(g.comment) : `<div class="qc-quote td-qa-comment">${esc(g.comment)}</div>`) : '')
    + `<button type="button" class="btn-m3-text td-qa-card" data-td-card="${esc(gateRef(g))}">そのときのカード</button>`
    + '</div></div>';
}

/* The plan's steps, and where the plan stands. Steps are plain text, as the panel draws them. */
function tdPlanHtml(plan, waiting) {
  const steps = plan.steps || [];
  let h = steps.length
    ? `<ol class="td-steps">${steps.map(x => `<li>${esc(x.text)}${x.detail ? `<div class="tp-muted">${esc(x.detail)}</div>` : ''}</li>`).join('')}</ol>`
    : `<div>${esc(plan.title || '')}</div>${tdMuted('手順は書かれていません')}`;
  const status = waiting ? '承認待ち'
    : ['approve', 'choice'].includes(plan.decision) ? `承認済み（${when(plan.answeredAt)}）`
    : plan.decision === 'changes' ? '差し戻し済み' : '';
  if (status) h += tdMuted(status);
  return h;
}

/* The report of a task that has no plan: the latest result gate's sections, or its body. */
function tdFoundHtml(result) {
  if ((result.sections || []).length) return result.sections.map(s => `<h5>${esc(s.heading)}</h5><div class="body">${md(s.body)}</div>`).join('');
  return result.body ? `<div class="body">${md(result.body)}</div>` : '';
}

/* The 概要 group. `all` is every gate of the task, oldest first. `ctx`: isWaiting(g), plan (the plan gate shown, or null), issueNumber,
   nowText, waiting (the gates asking a person now, oldest first). A row with nothing to say is left out, except 依頼, 質問と回答 and いま. */
function taskOverviewHtml(task, all, ctx = {}) {
  const isWaiting = ctx.isWaiting || (() => false);
  const plan = ctx.plan || null;
  const waiting = ctx.waiting || [];
  const rows = [];
  // 依頼
  let req = task.body ? `<div class="qc-quote">${md(task.body)}</div>`
    : task.issueSnapshot?.title ? `<div class="qc-quote"><b>${esc(task.issueSnapshot.title)}</b></div>`
    : '<span class="tp-muted">依頼文はありません</span>';
  if (task.issueSnapshot?.body) {
    req += `<div><button type="button" class="btn-m3-text" data-td-issue>${esc(ctx.issueNumber ? `Issue #${ctx.issueNumber} の本文` : 'Issue の本文')}</button></div>`;
  }
  rows.push(tdRow('person', '依頼', req));
  if (plan) {
    if (plan.goal) rows.push(tdRow('flag', 'ゴール', `<div class="body">${md(plan.goal)}</div>`));
    rows.push(tdRow('format_list_numbered', '計画', tdPlanHtml(plan, isWaiting(plan))));
  } else {
    const result = [...all].reverse().find(g => g.kind === 'result');
    const found = result ? tdFoundHtml(result) : '';
    if (found) rows.push(tdRow('search', '調べたこと', found));
  }
  // 質問と回答
  const asked = all.filter(g => g.wait !== false && !isWaiting(g) && g.decision)
    .sort((a, b) => (a.answeredAt || '').localeCompare(b.answeredAt || ''));
  rows.push(tdRow('forum', '質問と回答', asked.length ? asked.map(tdAnswerHtml).join('') : '<span class="tp-muted">まだありません</span>'));
  // いま
  let now = ctx.nowText ? `<div>${esc(ctx.nowText)}</div>` : '';
  for (const g of waiting) now += tdMuted(`「${kindOf(g.kind)[0]}」 ${tdQuestionOf(g)}`);
  if (waiting.length) now += tdMuted('答えるのは、ターミナルの上の質問か TODO からです');
  rows.push(tdRow('schedule', 'いま', now || '<span class="tp-muted">—</span>'));
  return rows.join('');
}

/* ── 変更（差分・動作確認） ── */

/* The panel's own cards for the review gate (`ctx.review`) and the check gate (`ctx.check`), under a line of facts. The cards carry the
   panel's 拡大 buttons and the like; the view half takes those out. */
function taskChangesHtml(task, all, ctx = {}) {
  const review = ctx.review || null;
  const check = ctx.check || null;
  const pr = task.pr ? prRefNumber(task.pr) : '';
  const facts = [...(review?.facts || []), ...(pr ? [`PR #${pr}`] : [])];
  let h = facts.length ? `<div class="qc-chips">${facts.map(f => `<span class="qc-chip">${esc(f)}</span>`).join('')}</div>` : '';
  if (review) h += roundsCardHtml(review) + findingsCardHtml(review) + filesTableHtml(review) + diffCardHtml(review);
  if (check) {
    h += commandsCardHtml(check) + manualCardHtml(check);
    if (check.run) h += `<div class="panel"><h3>動かし方</h3><div class="diff"><div>${esc(check.run).split('\n').join('</div><div>')}</div></div></div>`;
  }
  return h || '<span class="tp-muted">まだ変更はありません</span>';
}

/* ── 経過 ── */

const taskHistoryHtml = (task, all) => timelineHtml(task, all);

/* ── 詳細 ── */

function taskDetailKvHtml(task, { agent = '' } = {}) {
  const mono = v => `<span class="mono2">${esc(v)}</span>`;
  const rows = [];
  if (task.worktree) rows.push(['worktree', mono(task.worktree)]);
  if (task.branch) rows.push(['ブランチ', mono(task.branch) + (task.base ? esc(`（${task.base} から）`) : '')]);
  rows.push(['完了条件', esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')]);
  rows.push(['止める所', esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt || '—')]);
  if (task.instruction) rows.push(['申し送り', `<span class="td-pre">${esc(task.instruction)}</span>`]);
  if (agent) rows.push(['エージェント', esc(agent)]);
  rows.push(['タスクID', mono(task.id)]);
  rows.push(['作成', esc(`${when(task.createdAt)}（${ago(task.createdAt)}）`)]);
  return `<dl class="td-kv">${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('')}</dl>`;
}

/* ── the whole ── */

/* The four groups for `tocDialogHtml`. `ctx`: what `taskOverviewHtml` and `taskChangesHtml` take, and `agent`. */
function taskDetailGroups(task, all, ctx = {}) {
  return [
    { key: 'over', icon: 'description', label: '概要', html: taskOverviewHtml(task, all, ctx) },
    { key: 'change', icon: 'difference', label: '変更（差分・動作確認）', html: taskChangesHtml(task, all, ctx) },
    { key: 'log', icon: 'timeline', label: '経過', html: taskHistoryHtml(task, all) },
    { key: 'info', icon: 'info', label: '詳細', html: taskDetailKvHtml(task, { agent: ctx.agent }) },
  ];
}
