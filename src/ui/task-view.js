// ── the full view of one task ─────────────────────────────────────────

const TASK_TABS = [['overview', '概要'], ['review', 'コードレビュー'], ['check', '動作確認'], ['history', '経過']];
/* The tab a gate of each kind is read in. The other kinds are the hub's or a question, and
   show only in 経過. */
const TAB_OF_KIND = { plan:'overview', diff:'review', verify:'check' };
const KIND_OF_TAB = { overview:'plan', review:'diff', check:'verify' };
const DECISION = { approve:'承認した', changes:'修正を指示した', reject:'却下した', choice:'案を選んだ',
                   ack:'了解した', ask:'追加で聞いた', answer:'答えた', closed:'解決済みとして閉じた' };

/* `pick` is the gate a tab shows when a person chose one from 経過; otherwise a tab shows the
   one waiting, else the latest. */
let taskView = { id: null, tab: 'overview', pick: {} };

function openTask(id, tab = 'overview', gateId = null) {
  taskView = { id, tab, pick: gateId ? { [tab]: gateId } : {} };
  // Back on the board, the drawer is open on the task that was being read.
  selectedTaskId = id;
  setView('task');
}

function backToBoard() {
  setView('board');
}

/* What a task's gates left in the archive, read when its view opens rather than on every
   poll: the archive only grows. Read again when a gate of the task has been answered since,
   which is when it can have changed. */
const histories = {};
const historyFailed = new Set();
function historyOf(task) {
  const key = `${task.gateAnsweredAt || ''}|${openGate(task)?.id || ''}|${task.status}`;
  let entry = histories[task.id];
  if (!entry || entry.key !== key) {
    entry = histories[task.id] = { key, answered: entry?.answered || [], records: entry?.records || [], loaded: !!entry?.loaded };
    const mine = entry;
    api(`/api/tasks/${encodeURIComponent(task.id)}/history`).then(data => {
      if (histories[task.id] !== mine) return;
      mine.answered = data.answered || [];
      mine.records = data.records || [];
      mine.loaded = true;
      historyFailed.delete(task.id);
      if (view === 'task' && taskView.id === task.id) redrawTaskView();
    }).catch(e => {
      // Asked for again on the next redraw, keeping what was read before on screen meanwhile.
      if (histories[task.id] === mine) mine.key = null;
      // Said once per run of failures, so the retries below do not push the log of what was
      // done off the footer.
      if (!historyFailed.has(task.id)) note(`経過を取得できませんでした: ${e.message}`, true);
      historyFailed.add(task.id);
      // Nothing else redraws a quiet board, so the view asks again itself — while no comment
      // is being typed, since a redraw would cut an IME composition short.
      setTimeout(() => {
        if (view === 'task' && taskView.id === task.id) redrawTaskView();
      }, 30000);
    });
  }
  return entry;
}

/* Every gate of a task, oldest first: answered, kept as records, and waiting now. A live
   task's records come from /api/state, which is polled, so a send-back shows at once. */
function gatesOf(task) {
  const h = historyOf(task);
  const byId = new Map();
  const add = g => g && byId.set(g.id, g);
  h.answered.forEach(add);
  add(task.approvedPlan);
  (task.records || h.records).forEach(add);
  (state.gates || []).filter(g => g.task === task.id).forEach(add);
  // Same-second ties go by the sequence at the end of the id, as `recordsOf` orders them, so
  // the latest of a kind is the one claimed last.
  return [...byId.values()].sort((a, b) =>
    (a.openedAt || '').localeCompare(b.openedAt || '') || claimSeq(a) - claimSeq(b));
}

/* The order a gate or record was claimed in within its second: `…-diff-2`, `…-record-2`, and
   none for the first. */
const claimSeq = g => +(/-(\d+)$/.exec(g.id)?.[1] || 1);

const isWaiting = g => (state.gates || []).some(x => x.id === g.id);

/* The gate a tab shows: the one picked from 経過, else the one waiting, else the latest. */
function gateForTab(task, tab, all) {
  const picked = taskView.pick[tab] && all.find(g => g.id === taskView.pick[tab]);
  if (picked) return picked;
  const ofKind = all.filter(g => g.kind === KIND_OF_TAB[tab]);
  return ofKind.find(isWaiting) || ofKind[ofKind.length - 1] || null;
}

/* `20260922T041233Z` in the reader's own time: 9/22 13:12. */
function when(stamp) {
  const secs = stampSecs(stamp);
  if (secs == null) return stamp || '';
  const d = new Date(secs * 1000);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/* The files a unified diff touches, with the lines each gained and lost. */
function filesOf(diff) {
  const files = [];
  let cur = null, inHeader = false;
  for (const l of (diff || '').split('\n')) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(l);
    if (m) { cur = { path: m[2], add: 0, del: 0 }; files.push(cur); inHeader = true; continue; }
    if (l.startsWith('@@')) { inHeader = false; continue; }
    // A removed line that begins `--` is content, not a header: headers end at the first hunk.
    if (!cur || inHeader) continue;
    if (l.startsWith('+')) cur.add++;
    else if (l.startsWith('-')) cur.del++;
  }
  return files;
}

/* One line saying where a gate stands: waiting, recorded, or answered and how. */
function gateStatusHtml(g) {
  if (isWaiting(g)) return `<span class="state warn" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">hourglass_empty</span><span>${ago(g.openedAt)}から人の判定を待っている</span></span>`;
  if (g.wait === false) {
    const sent = (g.answers || []).length;
    return `<span class="state good" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-success);">check_circle</span><span>${ago(g.openedAt)}に記録 — worker は止まらずに進んだ${sent ? `（差し戻し ${sent}回）` : ''}</span></span>`;
  }
  if (g.decision) {
    const bad = ['changes', 'reject'].includes(g.decision);
    return `<span class="state ${bad ? 'bad' : 'good'}" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:${bad ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-success)'};">${bad ? 'replay' : 'check_circle'}</span>` +
      `<span title="${esc(when(g.answeredAt))}">${ago(g.answeredAt)}に人が${esc(DECISION[g.decision] || g.decision)}</span></span>` +
      (g.comment ? ` <span style="color:var(--md-sys-color-on-surface-variant)">— ${esc(g.comment)}</span>` : '');
  }
  return '';
}

/* The head of a tab showing one gate: its title, where it stands, and a way to see the others
   of its kind when there are several. */
function gateHeadHtml(g, all) {
  const same = all.filter(x => x.kind === g.kind);
  let h = `<div class="panel"><div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">` +
    `<b style="flex:1;min-width:0;overflow-wrap:anywhere">${esc(g.title)}</b>` +
    `<span class="mono2" style="color:var(--md-sys-color-outline)">${esc(when(g.openedAt))}</span></div>` +
    `<div style="margin-top:4px">${gateStatusHtml(g)}</div>`;
  if (stopWhy(g).length) {
    h += `<div class="state${stopBad(g) ? ' bad' : ''}" style="margin-top:6px;white-space:normal;display:flex;align-items:center;gap:6px;"><span class="material-symbols-outlined" style="font-size:16px;color:${stopBad(g) ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-warning)'};">${stopBad(g) ? 'error' : 'warning'}</span><span>止めた理由: ${esc(stopWhy(g).join(' / '))}</span></div>`;
  }
  if (same.length > 1) {
    h += `<div style="margin-top:8px;display:flex;gap:6px;flex-wrap:wrap;align-items:center">` +
      `<span style="color:var(--md-sys-color-outline);font-size:12px">このタスクの${esc(kindOf(g.kind)[0])} ${same.length}件:</span>` +
      same.map((x, i) => `<button type="button" class="chip${x.id === g.id ? ' good' : ''}" data-pick="${esc(x.id)}">` +
        `${i + 1}. ${esc(when(x.openedAt))}${x.wait === false ? ' 記録' : isWaiting(x) ? ' 待ち' : ''}</button>`).join('') + `</div>`;
  }
  return h + `</div>`;
}

/* The three frames, for a gate read in a task's view. */
function framesHtml(g) {
  let h = '';
  if (g.facts?.length) h += `<div class="panel"><h3><span class="material-symbols-outlined" style="font-size:18px;">info</span><span>事実</span></h3><ul>${g.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul></div>`;
  if (g.focus) h += `<div class="panel"><h3><span class="material-symbols-outlined" style="font-size:18px;">visibility</span><span>確認してほしい点</span></h3><div class="body frame-focus">${md(g.focus)}</div></div>`;
  if (g.unsure) h += `<div class="panel"><h3><span class="material-symbols-outlined" style="font-size:18px;">help</span><span>迷っていること</span></h3><div class="body">${md(g.unsure)}</div></div>`;
  return h;
}

/* The part of a tab a person acts on, when the gate shown can still take an answer: a gate
   waiting now, or a live task's record. A finished task's records have nowhere to go back to. */
function actHtml(g) {
  if (isWaiting(g) || (g.wait === false && recordById(g.id))) return decideHtml(g);
  return '';
}

/* A URL a link may point at: http or https only. The issue URL is typed into a form and
   stored as given, and `esc` keeps it from breaking the markup but not from being a
   `javascript:` link. */
const httpUrl = u => /^https?:\/\//i.test(u || '') ? u : null;
// The number at the end of an issue URL, ignoring a query, a fragment, or a trailing slash.
function issueNumberOf(url) {
  try { return new URL(url).pathname.split('/').filter(Boolean).pop() || ''; } catch { return ''; }
}
// The number of a pull request URL, also when it points at a tab of it (`…/pull/82/files`).
function prNumberOf(url) {
  try { return /\/pull\/(\d+)/.exec(new URL(url).pathname)?.[1] || ''; } catch { return ''; }
}

function overviewTab(task, all) {
  const plans = all.filter(g => g.kind === 'plan');
  const plan = gateShownIn(task, 'overview', all);
  let h = '';

  // Where each came from: the plan's own words when the worker wrote them, otherwise the
  // request as it was handed over.
  const issue = httpUrl(task.issueUrl);
  const source = issue
    ? `Issue <a href="${esc(issue)}" target="_blank" rel="noopener noreferrer">${esc(issue)}</a>`
    : task.issueUrl ? `Issue ${esc(task.issueUrl)}` : '依頼文';
  const fromPlan = plan && `計画「${esc(plan.title)}」— worker が${source}を読んで書いたもの`;
  h += `<div class="panel"><h3>問題</h3>`;
  if (plan?.problem) h += `<div class="body">${md(plan.problem)}</div><div class="source">出典: ${fromPlan}</div>`;
  else if (task.body) h += `<div class="body">${md(task.body)}</div><div class="source">出典: 渡したときの依頼文（計画に problem がまだ無い）</div>`;
  else h += `<div style="color:var(--muted)">まだ書かれていない${task.issueUrl ? ` — ${source}` : ''}</div>`;
  h += `</div><div class="panel"><h3>ゴール</h3>`;
  if (plan?.goal) h += `<div class="body">${md(plan.goal)}</div><div class="source">出典: ${fromPlan}</div>`;
  else h += `<div style="color:var(--muted)">計画に goal がまだ無い</div>`;
  h += `</div>`;

  if (task.instruction) {
    h += `<div class="panel" style="border-left: 3px solid var(--md-sys-color-primary, #6750A4);">` +
      `<h3>エージェントへの申し送り（指示）</h3>` +
      `<div class="body" style="white-space:pre-wrap;font-size:13.5px;line-height:1.6;">${esc(task.instruction)}</div>` +
      `<div class="source">キュー投入時の指示</div></div>`;
  }

  h += `<div class="panel"><h3>計画</h3>`;
  if (!plan) {
    h += `<div style="color:var(--muted)">計画はまだ出ていない</div></div>`;
  } else {
    // Who approved it: a plan gate is answered only by a person, on the board or with
    // `adj gate answer`; the gate does not record which one.
    const approved = ['approve', 'choice'].includes(plan.decision);
    h += `<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><b>${esc(plan.title)}</b></div>` +
      `<div style="margin-top:4px">${isWaiting(plan) ? `<span class="state warn" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">hourglass_empty</span><span>${ago(plan.openedAt)}から承認待ち</span></span>`
        : approved ? `<span class="state good" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-success);">check_circle</span><span title="${esc(when(plan.answeredAt))}">${esc(when(plan.answeredAt))}（${ago(plan.answeredAt)}）に人が${esc(DECISION[plan.decision])}</span></span>` +
          (plan.comment ? ` <span style="color:var(--md-sys-color-on-surface-variant)">— ${esc(plan.comment)}</span>` : '')
        : gateStatusHtml(plan)}</div>`;
    if (plans.length > 1) {
      h += `<div style="margin-top:8px;display:flex;gap:6px;flex-wrap:wrap;align-items:center">` +
        `<span style="color:var(--muted);font-size:12px">計画 ${plans.length}件:</span>` +
        plans.map((x, i) => `<button type="button" class="chip${x.id === plan.id ? ' good' : ''}" data-pick="${esc(x.id)}">` +
          `${i + 1}. ${esc(when(x.openedAt))}${isWaiting(x) ? ' 待ち' : x.decision ? ` ${esc(DECISION[x.decision] || x.decision)}` : ''}</button>`).join('') + `</div>`;
    }
    if (plan.focus) h += `<div class="body frame-focus" style="margin-top:10px">${md(plan.focus)}</div>`;
    h += `</div>`;
    if (plan.facts?.length) h += `<div class="panel"><h3>事実</h3><ul>${plan.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul></div>`;
    if (plan.body) h += `<div class="panel"><h3>報告</h3><div class="body">${md(plan.body)}</div></div>`;
    h += choicesHtml(plan, isWaiting(plan));
    if (plan.unsure) h += `<div class="panel"><h3>迷っていること</h3><div class="body">${md(plan.unsure)}</div></div>`;
    if (plan.decided) h += `<div class="panel"><h3>決定事項</h3><div class="body">${md(plan.decided)}</div></div>`;
    h += actHtml(plan);
  }

  const rows = [
    ['タスクID', `<span class="mono2">${esc(task.id)}</span>`],
    ['完了条件', esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')],
    ['止める所', esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt)],
    ['着手設定', task.autoStart ? '確認なしで着手' : '着手前に確認が必要'],
  ];
  if (task.instruction) rows.push(['申し送り', `<span style="white-space:pre-wrap">${esc(task.instruction)}</span>`]);
  const link = u => httpUrl(u) ? `<a href="${esc(u)}" target="_blank" rel="noopener noreferrer" style="color:var(--accent)">${esc(u)}</a>` : esc(u);
  if (task.issueUrl) rows.push(['Issue', link(task.issueUrl)]);
  if (task.pr) rows.push(['PR', link(task.pr)]);
  if (task.parent) rows.push(['親タスク', esc(task.parent)]);
  if (task.branch) rows.push(['ブランチ', `<span class="mono2">${esc(task.branch)}</span>`]);
  if (task.base) rows.push(['分岐元', `<span class="mono2">${esc(task.base)}</span>`]);
  if (task.worktree) rows.push(['worktree', `<span class="mono2">${esc(task.worktree)}</span> <button type="button" class="iconbtn" title="${ideTitle()}" data-ide="${esc(task.worktree)}">IDE で開く</button>`]);
  rows.push(['作成', `${esc(when(task.createdAt))}（${ago(task.createdAt)}）`]);
  if (task.note) rows.push(['ノート', `<span style="white-space:pre-wrap">${esc(task.note)}</span>`]);
  h += `<div class="panel"><h3>詳細</h3><dl class="kv">${rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('')}</dl></div>`;
  return h;
}

function reviewTab(task, all) {
  const g = gateForTab(task, 'review', all);
  if (!g) return `<div class="empty-state">コードレビューはまだ無い。worker がセルフレビューを終えると、ここに出る。</div>`;
  // A gate waiting is judged on what the worker says about it, so the decision sits right under
  // that, above the rounds, the findings and the diff it can be checked against. A record's
  // send-back stays at the end, after what it would be sent back about.
  const waiting = isWaiting(g);
  let h = gateHeadHtml(g, all) + framesHtml(g) + (waiting ? actHtml(g) : '') + reviewPanels(g);
  if (g.body) h += `<div class="panel"><h3>報告</h3><div class="body">${md(g.body)}</div></div>`;
  const files = filesOf(g.diff);
  if (files.length) {
    h += `<div class="panel"><h3>ファイル ${files.length}件</h3><div class="body"><table>` +
      `<tr><th>ファイル</th><th>追加</th><th>削除</th></tr>` +
      files.map(f => `<tr><td><code>${esc(f.path)}</code></td><td style="color:var(--good)">+${f.add}</td><td style="color:var(--critical)">−${f.del}</td></tr>`).join('') +
      `</table></div></div>`;
  }
  if (g.decided) h += `<div class="panel"><details class="decided"><summary>決定事項</summary><div class="body">${md(g.decided)}</div></details></div>`;
  if (g.diff) h += `<div class="panel"><h3>差分</h3><div class="diff">${renderDiff(g.diff)}</div></div>`;
  return h + (waiting ? '' : actHtml(g));
}

function checkTab(task, all) {
  const g = gateForTab(task, 'check', all);
  if (!g) return `<div class="empty-state">動作確認はまだ無い。worker が verify を回すと、ここに出る。</div>`;
  let h = gateHeadHtml(g, all);
  if (isWaiting(g)) {
    h += `<div class="work">
      <button class="big" title="${ideTitle()}" data-ide="${esc(g.worktree)}">IDE で開く</button>
      <span class="mono2">${esc(g.worktree)}</span>
      <span style="color:var(--ink-2)">— 確認後、下のボタンで判定してください</span></div>`;
  }
  h += framesHtml(g) + checkPanels(g);
  if (g.body) h += `<div class="panel"><h3>報告</h3><div class="body">${md(g.body)}</div></div>`;
  if (g.run) h += `<div class="panel"><h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>`;
  if (g.decided) h += `<div class="panel"><details class="decided"><summary>決定事項</summary><div class="body">${md(g.decided)}</div></details></div>`;
  return h + actHtml(g);
}

/* All of one gate, for the kinds that have no tab of their own (a report, a question, the hub's
   gates): picked from 経過 and shown above it, so nothing a gate carried is left unreadable. */
function gateDetailHtml(g, all) {
  let h = `<div class="panel" style="border-color:var(--accent)"><div style="display:flex;gap:8px;align-items:center">` +
    `<span class="tag"><span class="dot" style="background:${esc(kindOf(g.kind)[1])}"></span>${esc(kindOf(g.kind)[0])}</span>` +
    `<span style="flex:1"></span><button type="button" class="iconbtn" data-unpick>閉じる</button></div></div>`;
  h += gateHeadHtml(g, all) + framesHtml(g) + choicesHtml(g, isWaiting(g)) + reviewPanels(g) + checkPanels(g);
  if (g.body) h += `<div class="panel"><h3>報告</h3><div class="body">${md(g.body)}</div></div>`;
  if (g.run) h += `<div class="panel"><h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>`;
  if (g.decided) h += `<div class="panel"><h3>決定事項</h3><div class="body">${md(g.decided)}</div></div>`;
  if (g.diff) h += `<div class="panel"><h3>差分</h3><div class="diff">${renderDiff(g.diff)}</div></div>`;
  return h + actHtml(g);
}

/* In time order: what waited on a person, what was only recorded, and what people did. The
   worker's phase is not kept as a history — only the one it is in now — so it closes the
   list rather than running through it. */
function historyTab(task, all) {
  const events = [];
  const push = (stamp, html, gateId) => events.push({ at: stampSecs(stamp) ?? 0, stamp, html, gateId });
  push(task.createdAt, `<div>タスクを作成</div><div class="who">${esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '')} · ${esc(STOP_AT[task.stopAt || 'plan'] || '')}</div>`);
  for (const g of all) {
    const [label, colour] = kindOf(g.kind);
    const tag = `<span class="tag"><span class="dot" style="background:${esc(colour)}"></span>${esc(label)}</span>`;
    const title = `<button type="button" class="linkish" data-open="${esc(g.id)}">${esc(g.title)}</button>`;
    const why = stopWhy(g).length ? `<div class="who"${stopBad(g) ? ' style="color:var(--critical)"' : ''}>止めた理由: ${esc(stopWhy(g).join(' / '))}</div>` : '';
    const opened = g.wait === false ? 'worker が記録して、止まらずに進んだ'
      : isWaiting(g) ? 'worker が人を待っている' : 'worker が人を待った';
    let extra = '';
    if (g.kind === 'diff' || g.kind === 'verify') {
      const [text, tone] = recordSummary(g);
      extra = ` <span class="${tone === 'bad' ? 'state bad' : tone === 'good' ? 'state good' : ''}">${esc(text)}</span>`;
    }
    push(g.openedAt, `<div>${tag} ${title}${extra}</div><div class="who">${opened}</div>${why}`, g.id);
    if (g.answeredAt && g.decision) {
      push(g.answeredAt, `<div>人が${esc(DECISION[g.decision] || g.decision)} — ${esc(label)}「${esc(g.title)}」</div>` +
        (g.comment ? `<div class="who">${esc(g.comment)}</div>` : ''), g.id);
    }
    for (const a of g.answers || []) {
      push(a.answeredAt, `<div>人が差し戻した — ${esc(label)}の記録「${esc(g.title)}」</div>` +
        (a.comment ? `<div class="who">${esc(a.comment)}</div>` : ''), g.id);
    }
  }
  events.sort((a, b) => a.at - b.at);
  const worker = ['dispatched', 'pr'].includes(task.status) ? workerOf(task) : null;
  const picked = taskView.pick.history && all.find(g => g.id === taskView.pick.history);
  let h = picked ? gateDetailHtml(picked, all) : '';
  h += `<div class="panel"><h3>経過</h3><ol class="timeline">` + events.map(e =>
    `<li><span class="at" title="${esc(ago(e.stamp))}">${esc(when(e.stamp))}</span><div class="what">${e.html}</div></li>`).join('');
  if (worker && worker.present && worker.phase) {
    const mins = phaseMinutes(worker);
    h += `<li><span class="at">いま</span><div class="what"><div>worker は${esc(PHASE_LABEL[worker.phase] || worker.phase)}` +
      `${mins != null ? `（${minutesLabel(mins)}前から）` : ''}</div></div></li>`;
  }
  h += `</ol>`;
  if (!histories[task.id]?.loaded) h += `<div class="source">回答済みのものを読み込んでいる…</div>`;
  return h + `</div>`;
}

/* A redraw that came while a comment was being typed in the task view, held until the box is
   left. Replacing the markup under the box cuts an IME composition short even when the text
   is put back, and the view is redrawn on every change of state, not once a minute. */
let taskViewHeld = false;
function redrawTaskView() {
  if (view === 'task' && document.activeElement?.matches('#task-view .gate-comment')) {
    taskViewHeld = true;
    return;
  }
  renderTaskView();
}
document.getElementById('task-view').addEventListener('focusout', e => {
  if (!taskViewHeld || !e.target.matches('.gate-comment')) return;
  // After the focus has moved: a click on a button in the view redraws through its own handler,
  // and this one should not draw over it with the old state.
  setTimeout(() => {
    if (!taskViewHeld || document.activeElement?.matches('#task-view .gate-comment')) return;
    taskViewHeld = false;
    renderTaskView();
  });
});

let taskViewShown = null;
function renderTaskView() {
  taskViewHeld = false;
  const root = document.getElementById('task-view');
  if (view !== 'task') return;
  const rail = root.querySelector('.rail');
  const main = root.querySelector('.tv-main');
  const task = (state.tasks || []).find(t => t.id === taskView.id);
  if (!task) {
    rail.classList.add('hidden');
    root.classList.remove('with-rail');
    main.innerHTML = `<div class="tv-top"><div class="row"><button type="button" class="btn-m3-tonal" data-back style="padding:6px 14px;border-radius:var(--md-shape-corner-full);display:inline-flex;align-items:center;gap:6px;"><span class="material-symbols-outlined" style="font-size:18px;">arrow_back</span><span>ボード</span></button></div></div>` +
      `<div class="empty-state">このタスクは見つからない。${state.tasks?.length ? '取り消されたか、別のリポジトリのもの' : '読み込み中'}なのかもしれない。</div>`;
    main.querySelector('[data-back]').addEventListener('click', backToBoard);
    return;
  }
  // Replaced rather than pushed: nothing listens for the back button, and a tab switch that
  // left an entry would make it change the address without changing the page.
  history.replaceState(null, '', `#task/${encodeURIComponent(task.id)}/${taskView.tab}`);

  // The queue beside the task only while the task is on it: otherwise the list is about
  // something else, and the page is better spent on the task.
  const onQueue = columnOf(task) === 'attention';
  root.classList.toggle('with-rail', onQueue);
  rail.classList.toggle('hidden', !onQueue);
  if (onQueue) {
    const gates = state.gates || [];
    rail.innerHTML = `<h2>要対応 ${gates.length} 件</h2>`;
    for (const g of gates) {
      const [label, colour] = kindOf(g.kind);
      const b = document.createElement('button');
      b.className = 'item';
      b.setAttribute('aria-current', String(g.task === task.id));
      b.innerHTML = `<span class="tag"><span class="dot" style="background:${esc(colour)}"></span>${esc(label)}</span>
        <span class="t">${esc(g.title)}</span><span class="w">${ago(g.openedAt)}から待ち</span>`;
      b.onclick = () => judgeGate(g.id);
      rail.appendChild(b);
    }
  }

  const all = gatesOf(task);
  const shown = gateShownIn(task, taskView.tab, all);
  // Read before the tabs are drawn, so the one open now does not keep its dot.
  if (shown && shown.wait === false && recordById(shown.id)) markSeen(shown.id);
  const colObj = COLUMNS.find(c => c.id === columnOf(task));
  const worker = ['dispatched', 'pr'].includes(task.status) ? workerOf(task) : null;
  const count = kind => all.filter(g => g.kind === kind).length;
  const unreadIn = kind => all.some(g => g.kind === kind && g.wait === false && recordById(g.id) && isUnread(g));
  let h = `<div class="tv-top">
    <div class="row" style="display:flex;align-items:center;gap:12px;">
      <button type="button" class="btn-m3-tonal" data-back title="ボードに戻る (Esc)" style="padding:6px 14px;border-radius:var(--md-shape-corner-full);display:inline-flex;align-items:center;gap:6px;">
        <span class="material-symbols-outlined" style="font-size:18px;">arrow_back</span>
        <span>ボード</span>
      </button>
      <h1 style="flex:1;min-width:0;font-size:20px;font-weight:800;color:var(--md-sys-color-on-surface);margin:0;line-height:1.35;">${esc(task.title)}</h1>
      ${httpUrl(task.issueUrl) ? `<a href="${esc(task.issueUrl)}" target="_blank" rel="noopener noreferrer" style="font-size:13px;color:var(--md-sys-color-primary);text-decoration:none;font-weight:600;display:inline-flex;align-items:center;gap:4px;"><span>Issue #${esc(issueNumberOf(task.issueUrl))}</span><span class="material-symbols-outlined" style="font-size:16px;">open_in_new</span></a>` : ''}
    </div>
    <div class="row" style="font-size:12px;color:var(--md-sys-color-on-surface-variant);display:flex;gap:12px;align-items:center;flex-wrap:wrap;">
      <span class="m3-pill pill-blue">${esc(colObj ? colObj.label : task.status)}</span>
      ${worker && worker.present && worker.phase ? `<span style="font-size:12px;color:var(--md-sys-color-primary);font-weight:600;">${esc(PHASE_LABEL[worker.phase] || worker.phase)}${phaseMinutes(worker) != null ? `（${minutesLabel(phaseMinutes(worker))}）` : ''}</span>` : ''}
      ${task.worktree ? `<span>worktree: <code style="font-family:var(--font-mono);font-size:12px;">${esc(task.worktree.split('/').pop())}</code></span>` : ''}
      ${task.branch ? `<span>ブランチ: <code style="font-family:var(--font-mono);font-size:12px;">${esc(task.branch)}</code></span>` : ''}
      ${stuckOf(task) ? `<span class="m3-pill pill-critical" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:14px;">timer</span><span>${esc(stuckOf(task))}</span></span>` : ''}
      ${task.worktree ? `
        <span style="display:inline-flex;gap:6px;margin-left:auto;">
          <button class="m3-icon-button" style="padding:4px 10px;font-size:11px;" title="ターミナルのworkerタブを前面表示" data-focus="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:14px;">terminal</span>
            <span>端末</span>
          </button>
          <button class="m3-icon-button" style="padding:4px 10px;font-size:11px;" title="${ideTitle()}" data-ide="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:14px;">code</span>
            <span>IDE</span>
          </button>
        </span>
      ` : ''}
    </div>
    <div class="m3-segmented-tabs" role="tablist" style="margin-top:6px;margin-bottom:12px;">${TASK_TABS.map(([id, label]) => {
      const kind = KIND_OF_TAB[id];
      const n = kind && kind !== 'plan' ? count(kind) : 0;
      const icon = id === 'overview' ? 'description' : id === 'review' ? 'rate_review' : id === 'check' ? 'fact_check' : 'history';
      return `<button role="tab" id="tv-tab-${id}" class="m3-seg-tab${id === taskView.tab ? ' active' : ''}" aria-controls="tv-panel" data-tab="${id}" aria-selected="${id === taskView.tab}">` +
        `<span class="material-symbols-outlined" aria-hidden="true">${icon}</span>` +
        `<span>${label}</span>` +
        (n ? ` <span class="m3-tab-badge" style="background:var(--md-sys-color-surface-container-highest);color:var(--md-sys-color-on-surface);">${n}</span>` : '') +
        (kind && unreadIn(kind) ? '<span class="m3-tab-badge">新着</span>' : '') +
        `</button>`;
    }).join('')}</div>
  </div><div class="tv-body pane" id="tv-panel" role="tabpanel" aria-labelledby="tv-tab-${taskView.tab}" style="padding:24px 36px 80px;max-width:1080px;margin:0 auto;width:100%;">`;

  // A gate waiting on a person comes first whichever tab is open: it is the ball.
  const waiting = openGate(task);
  if (waiting) {
    const [label, colour] = kindOf(waiting.kind);
    const tab = TAB_OF_KIND[waiting.kind] || 'history';
    const here = tab === taskView.tab && gateShownIn(task, tab, all)?.id === waiting.id;
    h += `
      <div class="m3-card-attention-box" style="padding:14px 18px;margin-bottom:16px;display:flex;flex-direction:column;gap:8px;">
        <div style="display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:8px;">
          <div style="font-weight:800;font-size:14px;display:flex;align-items:center;gap:6px;">
            <span class="material-symbols-outlined" style="font-size:18px;">pending_actions</span>
            <span>【${esc(label)}】あなたの判定待ち</span>
          </div>
          <span style="font-size:11.5px;opacity:.9">${ago(waiting.openedAt)}から待ち (ID: ${esc(waiting.id)})</span>
        </div>
        ${stopWhy(waiting).length ? `<div style="font-size:12.5px;"><strong>止めた理由:</strong> <ul style="margin:4px 0 0;padding-left:18px;">${stopWhy(waiting).map(w => `<li${stopBad(waiting) ? ' style="color:var(--md-sys-color-error)"' : ''}>${esc(w)}</li>`).join('')}</ul></div>` : ''}
        ${waiting.focus ? `<div style="font-size:12.5px;background:rgba(0,0,0,.04);padding:8px 12px;border-radius:var(--md-shape-corner-xs);"><strong>確認してほしい点:</strong> ${md(waiting.focus)}</div>` : ''}
        ${here ? `<div style="font-size:12px;color:var(--md-sys-color-outline);display:flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:14px;">arrow_downward</span><span>判定パネルは${tab === 'review' ? 'この下、差分より前' : 'このページの下'}にあります</span></div>`
          : `<div><button type="button" class="btn-m3-tonal" data-judge="${esc(waiting.id)}" style="padding:4px 12px;font-size:12px;border-radius:var(--md-shape-corner-full);"><span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span><span>判定対象タブを開く</span></button></div>`}
      </div>
    `;
  }

  h += { overview: overviewTab, review: reviewTab, check: checkTab, history: historyTab }[taskView.tab](task, all);
  h += `</div>`;

  // Whether this is what was on screen: a comment is only carried across a redraw of the same one.
  const commentEl = main.querySelector('.gate-comment');
  const same = shown && taskViewShown === `${task.id}/${taskView.tab}/${shown.id}`;
  const commentVal = commentEl && same ? commentEl.value : '';
  const isCommentFocused = document.activeElement === commentEl;
  const scroll = taskViewShown?.startsWith(`${task.id}/${taskView.tab}/`) ? main.scrollTop : 0;
  taskViewShown = `${task.id}/${taskView.tab}/${shown?.id || ''}`;
  const openDetails = [...main.querySelectorAll('details')].map(d => d.open);

  main.innerHTML = h;
  if (same) main.querySelectorAll('details').forEach((d, i) => { if (openDetails[i] != null) d.open = openDetails[i]; });
  main.querySelector('[data-back]').addEventListener('click', backToBoard);
  main.querySelectorAll('[data-tab]').forEach(b => b.addEventListener('click', () => {
    taskView.tab = b.dataset.tab;
    renderTaskView();
  }));
  main.querySelectorAll('[data-pick]').forEach(b => b.addEventListener('click', () => {
    taskView.pick[taskView.tab] = b.dataset.pick;
    renderTaskView();
  }));
  main.querySelectorAll('[data-open]').forEach(b => b.addEventListener('click', () => {
    const g = all.find(x => x.id === b.dataset.open);
    if (!g) return;
    const tab = TAB_OF_KIND[g.kind] || 'history';
    taskView.tab = tab;
    taskView.pick[tab] = g.id;
    renderTaskView();
  }));
  main.querySelectorAll('[data-unpick]').forEach(b => b.addEventListener('click', () => {
    delete taskView.pick[taskView.tab];
    renderTaskView();
  }));
  main.querySelectorAll('[data-judge]').forEach(b => b.addEventListener('click', () => {
    const tab = TAB_OF_KIND[waiting.kind] || 'history';
    taskView.tab = tab;
    taskView.pick[tab] = waiting.id;
    renderTaskView();
  }));
  main.querySelectorAll('[data-focus]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('focus', b.dataset.focus)));
  main.querySelectorAll('[data-ide]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('ide', b.dataset.ide)));
  bindDecide(main);
  restoreComment(main, commentVal, isCommentFocused, scroll);
}

/* The gate a tab has on screen, if it shows one. */
function gateShownIn(task, tab, all) {
  if (tab === 'history') return (taskView.pick.history && all.find(g => g.id === taskView.pick.history)) || null;
  if (tab === 'overview') {
    // The plan being worked to is the one approved last; a plan waiting now is the one to judge.
    // /api/state names it only for a live task, so a finished one finds it the same way.
    const plans = all.filter(g => g.kind === 'plan');
    const approved = plans.filter(g => ['approve', 'choice'].includes(g.decision))
      .sort((a, b) => (a.answeredAt || '').localeCompare(b.answeredAt || '')).pop();
    return (taskView.pick.overview && all.find(g => g.id === taskView.pick.overview))
      || plans.find(isWaiting) || task.approvedPlan || approved || plans[plans.length - 1] || null;
  }
  return gateForTab(task, tab, all);
}

