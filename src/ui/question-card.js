/* ── The question card: the pure half (src/ui/question-dialog.js draws and wires it) ─────────────────────────────────────────────
   One gate as one card that reads top to bottom: あなたの依頼, 調べたこと, 計画, then the question as the heading, the choices one to a
   row, the comment box and the buttons. These functions take the gate and what surrounds it and return markup, touching neither the
   page nor the network, so `src/ui/tests/question-card.test.js` runs them under `node --test`. Loaded after util.js (`esc`, `md`,
   `kindOf`), decide.js (`BUTTONS`, `choicesHtml`, `gateRef`) and gate-evidence.js (`gateEvidenceParts`, `gateFocusLine`), and before
   my-work.js. What the card needs from the page comes in as `ctx`, never from a global that holds state, so a second board can
   build its own:
     { task, plan, read, nwo, key, issue: { number, url, hasBody }, now, ticked }
   `task` is the gate's task or null; `plan` the approved plan (the gate itself for a plan gate) or null; `read` the AI 下読み of
   the task's PR or null; `key` the page's key of the gate (`gateKey`), which `gateRef` stands in for; `ticked(ref)` the Set of
   manual checks ticked for a gate. A gate written before the steps and sections existed has the same four parts, filled from
   what it does have; a part with nothing to say is not drawn. */

const QUESTION_ASK_OF_KIND = {
  plan: 'この計画で進めていい？',
  diff: 'この差分で進めていい？',
  verify: '動かしてみて、問題なかった？',
  result: 'これで知りたいことは分かった？',
  dispatch: 'この読み取りで着手していい？',
  issue: 'この内容で起票していい？',
  relay: 'この指摘を Jules に回していい？',
  question: '答えてほしいこと',
};

const qcIcon = name => `<span class="material-symbols-outlined" aria-hidden="true">${name}</span>`;
const qcKey = (g, ctx) => ctx?.key ?? gateRef(g);
const qcChips = facts => (facts || []).length ? `<div class="qc-chips">${facts.map(f => `<span class="qc-chip">${esc(f)}</span>`).join('')}</div>` : '';

/* A part of the card: drawn only when it has something in it. */
const qcStage = (icon, title, inner, cls = '') => (inner ? `<section class="qc-stage${cls ? ` ${cls}` : ''}"><h3>${qcIcon(icon)}<span>${esc(title)}</span></h3>${inner}</section>` : '');
const qcSub = (icon, title) => `<h4 class="qc-sub">${qcIcon(icon)}<span>${esc(title)}</span></h4>`;

/* The files a unified diff touches, with the lines each gained and lost. `prParseDiff` reads the paths Git quotes, too. */
const qcDiffFiles = diff => prParseDiff(diff).map(f => ({ path: f.path, add: f.add, del: f.del }));

/* What the question says as its heading: what the worker asked, else the gate's title when a task gives the card its own title (or the
   title would be written twice), else a fixed question for the kind. HTML. */
function questionAskHtml(g, ctx = {}) {
  if (String(g.focus ?? '').trim()) return md(g.focus);
  if (ctx.task?.title && String(g.title ?? '').trim()) return esc(g.title);
  return esc(QUESTION_ASK_OF_KIND[g.kind] || QUESTION_ASK_OF_KIND.question);
}

/* 「前に決まっていること」: only on a diff or verify card, and only when the plan it carries out was answered with a choice or a comment. */
function questionEarlierHtml(g, ctx = {}) {
  const plan = ctx.task?.approvedPlan;
  if (!['diff', 'verify'].includes(g.kind) || !plan) return '';
  const comment = String(plan.comment ?? '').trim();
  const choice = plan.choice ? (plan.choices || []).find(c => c.id === plan.choice) : null;
  if (!choice && !comment) return '';
  const line = choice ? `計画で『${choice.label}』を選んだ` : '計画を承認した';
  return qcStage('history', '前に決まっていること', `<p class="qc-earlier">${esc(line)}${comment ? ` — ${esc(comment)}` : ''}</p>`);
}

/* The request in the person's own words, and the goal. */
function qcRequestHtml(g, ctx) {
  const task = ctx.task;
  const goal = ctx.plan?.goal;
  let h = '';
  if (task?.body) h += `<div class="qc-quote">${md(task.body)}</div>`;
  else if (task?.issueSnapshot?.title) h += `<div class="qc-quote"><b>${esc(task.issueSnapshot.title)}</b></div>`;
  if (goal) h += `<div class="qc-goal">${qcIcon('flag')}<div>${md(goal)}</div></div>`;
  if (task) {
    const url = httpUrl(task.issueUrl || ctx.issue?.url);
    const num = ctx.issue?.number;
    const label = `Issue${num ? ` #${num}` : ''} の本文`;
    if (ctx.issue?.hasBody ?? !!task.issueSnapshot?.body) h += `<button type="button" class="qc-link" data-qc-open="issue">${qcIcon('article')}<span>${esc(label)}</span></button>`;
    else if (url) h += `<a class="qc-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer">${qcIcon('open_in_new')}<span>${esc(`Issue${num ? ` #${num}` : ''} を GitHub で開く`)}</span></a>`;
  }
  return h ? qcStage('person', 'あなたの依頼', h) : '';
}

/* Where a gate's `body` is read: in 調べたこと when the gate has no sections of its own (`found`), behind a link when it has and the kind is a
   report (`link`), under 報告 in the plan part for the rest (`report`); never on a diff or verify card, which have their own parts. */
function qcBodyPlace(g) {
  if (['diff', 'verify'].includes(g.kind)) return 'none';
  if (!['result', 'question', 'dispatch', 'issue', 'relay'].includes(g.kind)) return 'report';
  if (!(g.sections || []).some(s => s.stage !== 'plan')) return 'found';
  return g.kind === 'result' ? 'link' : 'report';
}

/* The sections a card reads: on a diff or verify card, the approved plan's come first, for the gate itself rarely repeats them. */
const qcSections = (g, ctx) => [...(['diff', 'verify'].includes(g.kind) ? ctx.plan?.sections || [] : []), ...(g.sections || [])];

/* What was found: the problem, the sections the worker wrote under that stage, the facts. */
function qcFoundHtml(g, ctx, body) {
  let h = '';
  const problem = g.kind === 'plan' ? g.problem : g.problem || ctx.plan?.problem;
  if (problem) h += `${qcSub('error', '問題')}<div class="body">${md(problem)}</div>`;
  const found = qcSections(g, ctx).filter(s => s.stage !== 'plan');
  for (const s of found) h += `<h4>${esc(s.heading)}</h4><div class="body">${md(s.body)}</div>`;
  if (!['diff', 'verify'].includes(g.kind)) h += qcChips(g.facts);
  const place = qcBodyPlace(g);
  if (body && place === 'found') h += `<div class="body">${md(body)}</div>`;
  if (body && place === 'link') h += `<button type="button" class="qc-link" data-qc-open="report">${qcIcon('description')}<span>レポートを開く</span></button>`;
  return qcStage('manage_search', '調べたこと', h);
}

/* The steps, in the worker's order, with 済 on those done. */
function qcStepsHtml(steps, allDone) {
  return `<ol class="qc-steps">${steps.map(x => `<li><div class="qc-step-text">${esc(x.text)}${x.done === true || allDone ? '<span class="qc-chip ok">済</span>' : ''}</div>${x.detail ? `<div class="qc-step-detail">${esc(x.detail)}</div>` : ''}</li>`).join('')}</ol>`;
}

function qcFindingsHtml(g) {
  const findings = g.findings || [];
  const row = f => `<li><span class="m3-pill ${f.severity === 'must' ? 'pill-warn' : 'pill-blue'} qc-sev">${esc(f.severity)}</span>${f.location ? `<code>${esc(f.location)}</code> ` : ''}${esc(f.text)}${f.reason ? `<div class="qc-step-detail">${esc(f.reason)}</div>` : ''}</li>`;
  const open = findings.filter(f => f.outcome === 'open');
  const declined = findings.filter(f => f.outcome === 'declined');
  return (open.length ? qcSub('rule', `残っている指摘 ${open.length}件`) + `<ul class="qc-findings">${open.map(row).join('')}</ul>` : '')
    + (declined.length ? qcSub('block', `見送った指摘 ${declined.length}件`) + `<ul class="qc-findings">${declined.map(row).join('')}</ul>` : '');
}

function qcDiffHtml(g, ctx) {
  let h = '';
  const files = qcDiffFiles(g.diff);
  const add = files.reduce((n, f) => n + f.add, 0);
  const del = files.reduce((n, f) => n + f.del, 0);
  const fixed = (g.findings || []).filter(f => f.outcome === 'fixed').length;
  const chips = [];
  if (files.length) chips.push(`+${add} −${del}`, `${files.length} ファイル`);
  if ((g.reviewRounds || []).length) chips.push(`セルフレビュー ${g.reviewRounds.length} 巡`);
  if (fixed) chips.push(`修正済み ${fixed}`);
  h += qcChips([...chips, ...(g.facts || [])]);
  const evidence = gateEvidenceParts(g.diff, g.findings, ctx.read || null);
  if (evidence.html) {
    const note = evidence.annotated ? `<div class="qc-note">AI 下読みの注釈（${esc(prSha7(ctx.read.sha))} 時点）</div>` : '';
    h += qcSub('visibility', '見てほしいところ') + `${note}<div class="qc-excerpt">${evidence.html}</div>`;
  }
  return h + qcFindingsHtml(g);
}

function qcVerifyHtml(g, ctx) {
  let h = qcChips(g.facts);
  const commands = g.commands || [];
  if (commands.length) {
    h += `<div class="qc-chips">${commands.map(c => {
      const failed = c.result === 'fail';
      const retried = !failed && (c.attempts || 1) > 1;
      return `<span class="qc-chip ${failed ? 'ng' : 'ok'}">${failed ? '✗' : '✓'} <code>${esc(c.command)}</code>${retried ? ` ${esc(c.attempts)}回目で通過` : ''}</span>`;
    }).join('')}</div>`;
    for (const c of commands.filter(x => x.result === 'fail' && x.output))
      h += `<details open class="qc-output"><summary><code>${esc(c.command)}</code> の出力</summary><div class="diff">${renderDiff(c.output)}</div></details>`;
  }
  if ((g.manual || []).length) {
    const ticked = ctx.ticked ? ctx.ticked(gateRef(g)) : new Set();
    h += qcSub('visibility', '人が見る確認項目') + `<ul class="qc-manual">${g.manual.map((m, i) =>
      `<li><label><input type="checkbox" data-manual-gate="${esc(gateRef(g))}" data-manual-index="${i}"${ticked.has(i) ? ' checked' : ''}><span>${esc(m)}</span></label></li>`).join('')}</ul>`;
  }
  if (g.run) h += qcSub('play_arrow', '動かし方') + `<div class="diff">${renderDiff(g.run)}</div>`;
  return h;
}

/* The plan, or on a diff or verify card what was planned and done. */
function qcPlanHtml(g, ctx, body) {
  const after = ['diff', 'verify'].includes(g.kind);
  // A gate's own steps come first (a later gate may say how far the plan got); without them, the approved plan's.
  const own = g.kind === 'plan' || (g.steps || []).length > 0;
  const steps = (own ? g.steps : ctx.plan?.steps) || [];
  // An old worker wrote the steps in `decided`: on a plan gate with none of its own, that is the plan and is not shown a second time.
  const decidedIsPlan = g.kind === 'plan' && !steps.length && !!g.decided;
  // The same holds for the approved plan a diff or verify card carries out.
  const planDecided = !own && !steps.length ? ctx.plan?.decided || '' : '';
  const approved = !own && (steps.length > 0 || !!planDecided);
  // A diff or verify gate opens after the work, and no worker marks the approved steps done yet: until one does, every one reads as done.
  const allDone = after && approved && steps.length > 0 && !steps.some(x => 'done' in x);
  let h = '';
  if (approved) h += '<div class="qc-note">計画は承認済み</div>';
  if (steps.length) h += qcStepsHtml(steps, allDone);
  if (decidedIsPlan) h += `<div class="body">${md(g.decided)}</div>`;
  if (planDecided) h += `<div class="body">${md(planDecided)}</div>`;
  for (const s of qcSections(g, ctx).filter(x => x.stage === 'plan')) h += `<h4>${esc(s.heading)}</h4><div class="body">${md(s.body)}</div>`;
  if (g.kind === 'diff') h += qcDiffHtml(g, ctx);
  if (g.kind === 'verify') h += qcVerifyHtml(g, ctx);
  // What the gate says that no part above took: `body` when 調べたこと did not use it, and what was decided.
  if (body && qcBodyPlace(g) === 'report') h += qcSub('report', '報告') + `<div class="body">${md(body)}</div>`;
  if (g.decided && !decidedIsPlan) h += qcSub('task_alt', '決めたこと') + `<div class="body">${md(g.decided)}</div>`;
  if (g.unsure) h += qcSub('warning', '懸念点') + `<div class="qc-unsure">${md(g.unsure)}</div>`;
  return qcStage('format_list_numbered', after ? '計画とやったこと' : '計画', h);
}

/* The head of the card: what kind of question, where it comes from, how long it has waited, and whose task it is. */
function qcHeadHtml(g, ctx) {
  const [label, color] = kindOf(g.kind);
  const where = [ctx.nwo, ctx.issue?.number ? `#${ctx.issue.number}` : ''].filter(Boolean).join(' · ');
  // A card read as it was (`ctx.readOnly`) waits no more: it says when it was answered.
  const waited = ctx.readOnly ? (g.answeredAt ? `${when(g.answeredAt)}に回答` : '') : g.openedAt ? `${ago(g.openedAt)}から待ち` : '';
  return `<div class="qd-sub"><span class="qc-kind"><i style="background:${esc(color)}"></i>${esc(label)}</span>`
    + `${where ? `<span class="qc-where">${esc(where)}</span>` : ''}${waited ? `<span class="qc-where">${esc(waited)}</span>` : ''}</div>`
    + `<h2 class="qd-title">${esc(ctx.task?.title || g.title || label)}</h2>`;
}

/* The comment box and the buttons: what it takes to answer, once what is asked has been read. */
function qcFootHtml(g, ctx, draft) {
  const buttons = (g.options || []).map(o => (BUTTONS[o] || (() => ''))(g)).join('');
  const link = (attrs, icon, text, cls = 'm3-icon-button') => `<button type="button" class="${cls}" ${attrs}>${qcIcon(icon)}<span>${text}</span></button>`;
  return `<textarea class="gate-comment" aria-label="コメント" placeholder="ほかに伝えること（任意）">${esc(draft || '')}</textarea>`
    + `<div class="decide qc-decide">${buttons}<span class="qc-spacer"></span>`
    + (g.kind === 'diff' && g.diff ? link('data-qc-open="diff"', 'difference', '差分を全部見る') : '')
    + (g.kind === 'verify' && g.worktree ? link('data-qc-ide', 'code', 'IDE で開く') : '')
    + link('data-act="talk"', 'terminal', 'ターミナルで話す', 'm3-icon-button talk')
    + link('data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします"', 'done_all', '解決済みとして閉じる', 'btn-m3-text close')
    + '</div>';
}

/* The card in three pieces for a dialog: the head (kind, where from, the task's title), the body that scrolls, and the foot that stays.
   `ctx.draft` is the comment typed earlier. A record never opens as a card; an answered gate opens read-only (`ctx.readOnly`: the
   choices are drawn with nothing to pick and there is no foot), as the task detail shows what was asked at the time. */
function questionCardHtml(g, ctx = {}) {
  const body = g.body && g.body !== ctx.task?.body ? g.body : '';
  const parts = [
    qcRequestHtml(g, ctx),
    qcFoundHtml(g, ctx, body),
    qcPlanHtml(g, ctx, body),
    questionEarlierHtml(g, ctx),
    `<section class="qc-ask-stage"><div class="qc-ask" role="heading" aria-level="3">${questionAskHtml(g, ctx)}</div>${choicesHtml(g, !ctx.readOnly, { expand: false, bare: true })}</section>`,
  ];
  return { head: qcHeadHtml(g, ctx), body: parts.join(''), foot: ctx.readOnly ? '' : qcFootHtml(g, ctx, ctx.draft) };
}

/* The card as one article, for a pane that holds it with no dialog around it. */
function questionCardArticleHtml(g, ctx = {}) {
  const c = questionCardHtml(g, ctx);
  return `<article class="qc" data-qc-gate="${esc(qcKey(g, ctx))}"><header class="qd-head-main">${c.head}</header>${c.body}<footer class="qc-foot">${c.foot}</footer></article>`;
}

/* A question answered where it stands: the question in a line, the facts as chips, the choices (or the options) as buttons. The
   recommended choice is the filled one; with none recommended, the first. */
function quickRowHtml(g, ctx = {}) {
  // The whole question, not its first line: a quick row has no card to open for the rest. A list that has the card beside it
  // (`ctx.firstLine`) shows the first line only.
  const line = ctx.firstLine ? gateFocusLine(g.focus) || gateFocusLine(g.title) : mdPlain(g.focus).trim() || mdPlain(g.title).trim();
  const choices = g.choices || [];
  const lead = choices.findIndex(c => c.recommended);
  const buttons = choices.length
    ? choices.map((c, i) => `<button type="button" class="pick ${i === Math.max(lead, 0) ? 'btn-m3-primary' : 'btn-m3-tonal'}" data-choice="${esc(c.id)}">${esc(c.label)}</button>`).join('')
    : (g.options || []).map(o => (BUTTONS[o] || (() => ''))(g)).join('');
  return `<div class="qc-quick" data-qc-gate="${esc(qcKey(g, ctx))}">${qcIcon('bolt')}<b class="qc-quick-text">${esc(line)}</b>${qcChips(g.facts)}<span class="qc-spacer"></span>${buttons}</div>`;
}

/* The lines over a task's terminal: a quick question as its row, any other as a line that opens the card. `ctx` is the card's context, or a
   function from the gate to it. */
function questionStripHtml(gates, ctx = {}) {
  return (gates || []).map(g => {
    const c = typeof ctx === 'function' ? ctx(g) : ctx;
    if (g.quick) return quickRowHtml(g, c);
    const line = gateFocusLine(g.focus) || gateFocusLine(g.title);
    return `<div class="qc-strip">${qcIcon('front_hand')}<span class="qc-kind">${esc(kindOf(g.kind)[0])}</span><span class="qc-strip-text">${esc(line)}</span>`
      + `<button type="button" class="btn-m3-tonal" data-qc-open-gate="${esc(qcKey(g, c))}">開いて答える</button></div>`;
  }).join('');
}
