/* The task panel's renderers in src/ui/task-view.js and src/ui/decide.js are concatenations of
   smaller card renderers: the panel's output must be the pieces in order. The 「拡大」 dialog of a whole task
   (src/ui/card-dialog.js) draws from the same pieces: its model, index and follow. Run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const fn = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\(.*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));
const cst = (src, name) => cut(src, new RegExp(`^const ${name} = [^\\n]*\\n(?:  [^\\n]*\\n)*`, 'm'));

const tvSrc = read('task-view.js');
const decSrc = read('decide.js');
const dlgSrc = read('card-dialog.js');
const utilSrc = read('util.js');

const WAITING = [];
const ctx = vm.createContext({
  TextEncoder, TextDecoder,
  state: { gates: WAITING },
  BASE: 'main', baseOf: () => 'main',
  DONE_WHEN: { pr: 'PR作成まで' }, STOP_AT: { plan: '計画の承認だけ待つ' },
  KINDS: {}, ...{},
  ago: s => `ago(${s})`, when: s => `when(${s})`,
  md: s => `<p>${String(s).replace(/</g, '&lt;')}</p>`,
  stopWhy: g => g.stoppedBy || [], stopBad: () => false,
  ideTitle: () => 'ide', julesText: () => 'jules', parkOf: t => t.parked || null,
  parkText: p => p.reason, stampSecs: () => 1, parkAttrs: () => 'data-park',
  parentRowHtml: () => '', recordByRef: () => null,
  decideHtml: g => `<decide ${g.id}>`, gateDockHtml: g => `<dock ${g.id}>`,
  parkBannerHtml: () => '<park>', ghRowsHtml: t => `<rows ${t.id}>`,
  choicesHtml: (g, p) => g.options ? `<choices ${g.id} ${p}>` : '',
  manualChecked: () => new Set([1]),
  gateStatusHtml: () => '<status>',
  timelineHtml: () => '<ol class="timeline"><li><button data-open="d-1">t</button></li></ol>',
  sessionOfTask: () => SESSION.s, ensureLastMessage: s => { SESSION.asked.push(s); },
  lastMessageHtml: (a, s, opts = {}) => `<div class="tp-lastmsg" data-expanded="${!!opts.expanded}" data-for="${s?.id}">${a.lastMessage}</div>`,
  histories: {}, historyKey: t => t.id,
});
const SESSION = { s: null, asked: [] };
const load = (src) => vm.runInContext(src, ctx);
load([
  cut(utilSrc, /^const esc = [^\n]*\n/m),
  cut(utilSrc, /^const httpUrl = [^\n]*\n/m),
  cut(utilSrc, /^const kindOf = [^\n]*\n/m),
  cut(dlgSrc, /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  'const gateRef = g => g._slug ? `${g._slug}/${g.id}` : g.id;',
  'const DECISION = { approve: "承認した", changes: "差し戻した" };',
  'const renderDiff = d => esc(d);',
  cst(tvSrc, 'diffPending'), cst(tvSrc, 'gateOfPlan'),
  cut(tvSrc, /^const DIFF_LOADING = [^\n]*\n/m),
  cst(tvSrc, 'isWaiting'),
  cut(utilSrc, /^let viewData = null;\nconst stateNow = [^\n]*\nfunction withViewState[\s\S]*?\n\}\n/m),
  'const isGithubIssue = u => /issues\\/\\d+/.test(u || "");',
  'const issueNumberOf = u => /(\\d+)$/.exec(u)?.[1] || "";',
  fn(tvSrc, 'gateShownIn'), fn(tvSrc, 'gateForTab'), fn(tvSrc, 'filesOf'),
  'const KIND_OF_TAB = { overview: "plan", review: "diff", check: "verify" };',
  fn(tvSrc, 'actHtml'),
  fn(tvSrc, 'gateHeadHtml'), fn(tvSrc, 'gateStatusHtml'),
  ...['framesHtml', 'factsCardHtml', 'focusCardHtml', 'unsureCardHtml', 'stepsCardHtml', 'sectionsCardsHtml', 'reportCardHtml', 'filesTableHtml',
    'diffCardHtml', 'planSourceOf', 'problemCardHtml', 'issueBodyCardHtml', 'goalCardHtml',
    'instructionCardHtml', 'planHeadCardHtml', 'planGateCardsHtml', 'detailsKvHtml', 'planApprovalOf', 'taskPlanCardHtml', 'ownPlanOf', 'planProblemShown', 'planWrittenBy', 'overviewTab',
    'reviewTab', 'checkTab', 'gateDetailHtml'].map(n => fn(tvSrc, n)),
  cut(decSrc, /^const OUTCOME = [^\n]*\n/m),
  ...['cardDialogWaiting'].map(n => cst(dlgSrc, n)),
  ...['cardDialogAnsweredHeadHtml', 'cardDialogTaskGroups', 'cardDialogResolveTarget', 'cardDialogDockGate', 'cardDialogTaskHtml', 'cardDialogDockHtml', 'cardDialogActiveGroup']
    .map(n => cut(dlgSrc, new RegExp(`^function ${n}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'))),
  ...['gateWaitHeadHtml', 'gateJudgeHtml', 'reviewPanels', 'roundsCardHtml', 'findingsCardHtml',
    'checkPanels', 'commandsCardHtml', 'manualCardHtml'].map(n => fn(decSrc, n)),
].join('\n'));
vm.runInContext(read('pr-diff.js'), ctx);
const call = (name, ...a) => { ctx.__a = a; return vm.runInContext(`${name}(...__a)`, ctx); };
const wait = g => { WAITING.length = 0; WAITING.push(...g); };

const task = {
  id: 't-1', status: 'working', body: 'Do <b>the</b> thing', instruction: 'be careful', doneWhen: 'pr', stopAt: 'plan',
  issueUrl: 'https://github.com/o/r/issues/7', pr: 'https://github.com/o/r/pull/8', branch: 'b', base: 'main',
  worktree: '/w', createdAt: '2026-10-01T00:00:00Z', note: 'n',
  issueSnapshot: { url: 'https://github.com/o/r/issues/7', title: 'T <1>', body: 'issue body', truncated: true, fetchedAt: '2026-10-01T00:00:00Z' },
};
const plan = { id: 'p-1', kind: 'plan', title: 'Plan', problem: 'prob', goal: 'goal', focus: 'plan focus', facts: ['f1', 'f2'],
  body: 'plan report', options: [1], unsure: 'unsure?', decided: 'chose A', decision: 'approve', answeredAt: 's', openedAt: 'o' };
const plan2 = { ...plan, id: 'p-2', decision: undefined, answeredAt: undefined };
const diffGate = { id: 'd-1', kind: 'diff', title: 'Diff', problem: 'why', facts: ['a'], focus: 'look', unsure: 'hm', body: 'rep', openedAt: 'o',
  decided: 'x', stoppedBy: ['r1'],
  reviewRounds: [{ engine: 'e', must: 1, want: 2, scope: 0, falsePositives: 1 }],
  findings: [{ severity: 'must', outcome: 'open', text: 'bad <t>', location: 'a.rs:1' }, { severity: 'want', outcome: 'declined', text: 'x', reason: 'r' }],
  diff: 'diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-a\n+b\n' };
const bareGate = { id: 'v-1', kind: 'verify', title: 'Bare', openedAt: 'o' };
const verifyGate = { id: 'v-2', kind: 'verify', title: 'V', facts: ['z'], body: 'vrep', run: 'make\ntest', worktree: '/w', openedAt: 'o',
  commands: [{ command: 'make', result: 'fail', output: 'o\nk' }, { command: 'ok', result: 'pass', attempts: 2 }], manual: ['look', 'again'] };
const pendingDiff = { id: 'd-2', kind: 'diff', title: 'Pending', diffSize: 10, openedAt: 'o' };

test('framesHtml is the facts, the focus and the unsure cards in that order', () => {
  for (const g of [diffGate, bareGate, plan, { ...diffGate, facts: [], unsure: '' }]) {
    assert.strictEqual(call('framesHtml', g), call('factsCardHtml', g) + call('focusCardHtml', g) + call('unsureCardHtml', g));
  }
  assert.ok(call('framesHtml', diffGate).includes('data-expand="focus"'));
  assert.strictEqual(call('framesHtml', bareGate), '');
});

test('reviewPanels is the rounds and the findings, checkPanels the commands and the manual checks', () => {
  for (const g of [diffGate, bareGate, { ...diffGate, reviewRounds: [] }, { ...diffGate, findings: [] }]) {
    assert.strictEqual(call('reviewPanels', g), call('roundsCardHtml', g) + call('findingsCardHtml', g));
  }
  for (const g of [verifyGate, bareGate, { ...verifyGate, manual: [] }, { ...verifyGate, commands: [] }]) {
    assert.strictEqual(call('checkPanels', g), call('commandsCardHtml', g) + call('manualCardHtml', g));
  }
  assert.ok(call('roundsCardHtml', diffGate).includes('data-expand="rounds"'));
  assert.ok(call('findingsCardHtml', diffGate).includes('指摘 2件'));
  assert.ok(call('commandsCardHtml', verifyGate).includes('2回目で通過'));
  assert.ok(call('manualCardHtml', verifyGate).includes('data-manual-index="1" checked'));
  assert.strictEqual(call('roundsCardHtml', bareGate) + call('findingsCardHtml', bareGate), '');
});

test('overviewTab is its cards in order, with and without a plan, a hand-over form and the panel', () => {
  for (const [t, all, opts] of [
    [task, [plan], {}], [task, [plan, plan2], {}], [task, [], {}], [task, [plan], { panel: true }],
    [task, [plan], { panel: true, handForm: true }], [{ ...task, issueSnapshot: null, instruction: '' }, [plan2], {}],
    [{ ...task, body: '', issueUrl: '' }, [], { handForm: true }],
  ]) {
    wait(all.filter(g => g === plan2));
    const p = call('gateShownIn', t, 'overview', all, {});
    const expected = call('problemCardHtml', t, p) + call('issueBodyCardHtml', t) + call('goalCardHtml', t, p) +
      (opts.handForm ? '' : call('instructionCardHtml', t)) +
      call('planHeadCardHtml', p, all.filter(g => g.kind === 'plan')) + call('planGateCardsHtml', p) + call('detailsKvHtml', t, opts);
    assert.strictEqual(call('overviewTab', t, all, {}, opts), expected);
  }
  wait([]);
});

const ownPlan = { problem: 'own <problem>', goal: 'own goal', steps: [{ text: 'first <b>', detail: 'in a.rs' }, { text: 'second' }],
  decided: 'chose A', unsure: 'maybe B', revision: 2, gate: 'p-1', updatedAt: 'u1' };
const mark = (t, all) => (call('taskPlanCardHtml', t, all).match(/data-task-plan="(\w+)"/) || [])[1];
const count = (h, text) => h.split(text).length - 1;

test('overviewTab reads the plan from the task when it has one: no duplicate problem or goal, steps in order', () => {
  const t = { ...task, plan: ownPlan, approvedPlan: plan };
  wait([]);
  const h = call('overviewTab', t, [plan], {}, {});
  // The problem and the goal come from the task's plan, once, not from the gate.
  assert.strictEqual(count(h, 'own &lt;problem>'), 1);
  assert.strictEqual(count(h, 'own goal'), 1);
  assert.ok(!h.includes('<p>prob</p>') && !h.includes('<p>goal</p>'));
  assert.ok(h.includes('第2版') && h.includes('when(u1)'));
  assert.ok(h.includes('タスクの計画（第2版）'));
  // Steps are escaped, in order, with their detail; unsure shown, decided folded.
  assert.ok(h.indexOf('first &lt;b&gt;') < h.indexOf('second'));
  assert.ok(h.includes('in a.rs'));
  assert.ok(h.includes('<details class="decided"><summary>決定事項</summary>'));
  assert.ok(h.includes('maybe B'));
  // The gate that wrote it does not repeat the steps, the unsure or the decided; its head stays.
  assert.strictEqual(count(h, 'maybe B'), 1);
  assert.strictEqual(count(h, 'chose A'), 1);
  assert.strictEqual(count(h, 'class="steps"'), 1);
  assert.ok(h.includes('計画のゲート') && h.includes('plan focus'));
});

test('another gate than the one that wrote the task plan keeps its own steps', () => {
  const t = { ...task, plan: { ...ownPlan, gate: 'p-9' } };
  wait([]);
  const h = call('overviewTab', t, [plan], {}, {});
  assert.strictEqual(count(h, 'class="steps"'), 1); // the gate has no steps in this fixture
  const withSteps = { ...plan, steps: [{ text: 'gate step' }] };
  const h2 = call('overviewTab', t, [withSteps], {}, {});
  assert.strictEqual(count(h2, 'class="steps"'), 2);
  assert.ok(h2.includes('gate step'));
});

test('a task without a plan draws the overview from the plan gate as before', () => {
  wait([]);
  const h = call('overviewTab', task, [plan], {}, {});
  assert.ok(!h.includes('data-task-plan'));
  assert.ok(h.includes('<p>prob</p>') && h.includes('<p>goal</p>'));
  assert.ok(!h.includes('計画のゲート'));
  // No plan at all: today's empty state.
  const none = call('overviewTab', task, [], {}, {});
  assert.ok(none.includes('計画はまだ出ていない'));
});

test('a task plan written by hand has no gate card of its own', () => {
  const t = { ...task, plan: { ...ownPlan, gate: undefined } };
  wait([]);
  const h = call('overviewTab', t, [], {}, {});
  assert.strictEqual(mark(t, []), 'nogate');
  assert.ok(h.includes('ゲートなしで書いた計画'));
  assert.ok(!h.includes('計画はまだ出ていない') && !h.includes('計画のゲート'));
});

test('the approval mark follows the plan gate that wrote the task plan', () => {
  const t = gate => ({ ...task, plan: { ...ownPlan, gate } });
  // Approved: the approved plan's id.
  wait([]);
  assert.strictEqual(mark({ ...t('p-1'), approvedPlan: plan }, [plan]), 'approved');
  assert.ok(call('taskPlanCardHtml', { ...t('p-1'), approvedPlan: plan }, [plan]).includes('承認済み'));
  // Waiting now.
  wait([plan2]);
  assert.strictEqual(mark(t('p-2'), [plan2]), 'waiting');
  assert.ok(call('taskPlanCardHtml', t('p-2'), [plan2]).includes('承認待ち'));
  wait([]);
  // Kept as a record under Stop at none.
  const record = { id: 'r-1', kind: 'plan', wait: false, openedAt: 'o' };
  assert.strictEqual(mark(t('r-1'), [record]), 'recorded');
  assert.ok(call('taskPlanCardHtml', t('r-1'), [record]).includes('承認なしで記録'));
  // Answered but not approved, and a gate nobody can find.
  const sentBack = { id: 'p-3', kind: 'plan', decision: 'changes', openedAt: 'o' };
  assert.strictEqual(mark(t('p-3'), [sentBack]), 'unapproved');
  assert.strictEqual(mark(t('gone'), []), 'unapproved');
  assert.ok(call('taskPlanCardHtml', t('gone'), []).includes('未承認'));
  // A task with no plan draws nothing.
  assert.strictEqual(call('taskPlanCardHtml', task, [plan]), '');
});

test('detailsKvHtml: the panel leaves the Issue and PR to its top, the dialog options bring them back', () => {
  const rows = h => [...h.matchAll(/<dt>([^<]*)<\/dt>/g)].map(m => m[1]);
  assert.deepStrictEqual(rows(call('detailsKvHtml', task)),
    ['タスクID', '完了条件', '止める所', '着手設定', '申し送り', 'Issue', 'PR', 'ブランチ', '分岐元', 'worktree', '作成', 'ノート']);
  const panel = rows(call('detailsKvHtml', task, { panel: true, handForm: true }));
  assert.ok(!panel.includes('Issue') && !panel.includes('PR') && !panel.includes('申し送り') && panel.includes('置いている'));
  const dialog = rows(call('detailsKvHtml', task, { panel: true, refs: true, park: true }));
  assert.ok(dialog.includes('Issue') && dialog.includes('PR') && dialog.includes('置いている'));
  assert.ok(!rows(call('detailsKvHtml', { ...task, status: 'done' }, { panel: true, refs: true, park: true })).includes('置いている'));
  assert.ok(rows(call('detailsKvHtml', task, { park: true })).includes('置いている'));
});

test('reviewTab and checkTab are their cards in order', () => {
  wait([diffGate]);
  const rv = call('reviewTab', task, [diffGate], {});
  const expectedRv = call('gateHeadHtml', diffGate, [diffGate]) + call('framesHtml', diffGate) + call('actHtml', diffGate) +
    call('reviewPanels', diffGate) + call('reportCardHtml', diffGate) + call('filesTableHtml', diffGate) +
    rv.slice(rv.indexOf('<div class="panel" data-expand="decided"'), rv.indexOf('<div class="panel" data-expand="diff"')) +
    call('diffCardHtml', diffGate);
  assert.ok(rv.includes('<details class="decided">'));
  assert.strictEqual(rv, expectedRv);
  wait([]);
  const rec = call('reviewTab', task, [{ ...pendingDiff }], {});
  assert.ok(rec.endsWith(call('diffCardHtml', pendingDiff)));
  assert.strictEqual(call('diffCardHtml', pendingDiff), vm.runInContext('DIFF_LOADING', ctx));
  assert.strictEqual(call('diffCardHtml', bareGate), '');
  wait([verifyGate]);
  const ck = call('checkTab', task, [verifyGate], {});
  assert.ok(ck.indexOf(call('checkPanels', verifyGate)) < ck.indexOf(call('reportCardHtml', verifyGate)));
  wait([]);
});

test('gateJudgeHtml is the refs, the wait head and what follows, for a gate with and without facts', () => {
  wait([diffGate, bareGate]);
  for (const g of [diffGate, bareGate, { ...diffGate, wait: false }]) {
    const h = call('gateJudgeHtml', g, task);
    const head = call('gateWaitHeadHtml', g);
    assert.ok(h.startsWith(`<div class="rv-refs" data-rv-refs><rows t-1></div>${head}`));
    assert.ok(head.includes('rv-wait') && head.includes(`data-expand="wait"`));
    assert.strictEqual(call('gateJudgeHtml', g).startsWith(head), true);
  }
  const d = call('gateJudgeHtml', diffGate, task);
  assert.ok(d.includes(call('reviewPanels', diffGate)) && d.endsWith('<dock d-1>'));
  assert.ok(call('gateJudgeHtml', { ...diffGate, wait: false }, task).endsWith('<decide d-1>'));
  assert.ok(call('gateWaitHeadHtml', { ...diffGate, wait: false }).includes('記録'));
  wait([]);
});

/* The model of the dialog of a whole task. */
const dgate = (id, kind, at, extra = {}) => ({ id, kind, title: `T ${id}`, openedAt: at, ...extra });
const groupsOf = (all, t = task) => {
  wait(all.filter(g => g.waiting));
  return JSON.parse(JSON.stringify(call('cardDialogTaskGroups', t, all)));
};
const keysOf = gs => gs.map(g => g.key);
const cardOf = (gs, key) => gs.flatMap(g => g.cards).find(c => c.key === key);

test('the dialog reads in order: waiting gates oldest first, review, diff, plan, answered, 経過, 詳細', () => {
  const answeredPlan = dgate('p-1', 'plan', '2026-10-01T00:00:00Z', { ...plan, id: 'p-1', openedAt: '2026-10-01T00:00:00Z' });
  const wDiff = dgate('d-1', 'diff', '2026-10-03T00:00:00Z', { ...diffGate, openedAt: '2026-10-03T00:00:00Z', waiting: true, options: ['approve'] });
  const wVerify = dgate('v-2', 'verify', '2026-10-02T00:00:00Z', { ...verifyGate, openedAt: '2026-10-02T00:00:00Z', waiting: true, options: ['approve'] });
  const all = [answeredPlan, wVerify, wDiff];
  const gs = groupsOf(all);
  assert.deepStrictEqual(keysOf(gs), ['waiting:v-2', 'waiting:d-1', 'review', 'diff', 'plan', 'answered', 'history', 'details']);
  assert.deepStrictEqual(gs.slice(0, 2).map(g => g.badge), ['判断待ち', '判断待ち']);
  // Not the panel's 記録 list or 工程.
  assert.ok(!gs.some(g => /record|progress|step/.test(g.key)));
  // The waiting diff gate is the review's: its rounds and findings are in group 2, not again in its own group.
  assert.ok(!cardOf(gs, 'gate:d-1:rounds') && cardOf(gs, 'review:rounds') && cardOf(gs, 'review:findings').wide);
  assert.ok(cardOf(gs, 'review:diff').wide && cardOf(gs, 'review:files').wide);
  assert.ok(cardOf(gs, 'gate:v-2:head').wide && !cardOf(gs, 'gate:v-2:facts').wide);
  assert.ok(cardOf(gs, 'gate:v-2:commands') && cardOf(gs, 'gate:v-2:manual') && cardOf(gs, 'gate:v-2:run'));
  assert.strictEqual(cardOf(gs, 'gate:p-1').badge, '承認した');
  // What the plan gate carried is in its own card, not repeated in the plan group.
  const plans = gs.find(g => g.key === 'plan').cards.map(c => c.html).join('');
  for (const section of ['facts', 'unsure', 'report', 'choices']) assert.ok(!plans.includes(`data-expand="${section}"`), section);
  assert.ok(cardOf(gs, 'gate:p-1').html.includes('data-expand="facts"'));
  assert.ok(cardOf(gs, 'plan:issue').wide);
  wait([]);
});

test('the choices of every waiting gate can be picked, and a waiting group names its gate', () => {
  const a = dgate('a-1', 'question', '2026-10-01T00:00:00Z', { waiting: true, options: ['answer'], choices: [1], title: 'A' });
  const b = dgate('b-1', 'question', '2026-10-02T00:00:00Z', { waiting: true, options: ['answer'], choices: [1], title: 'B' });
  const all = [a, b];
  const gs = groupsOf(all);
  assert.ok(cardOf(gs, 'gate:a-1:choices').html.includes('<choices a-1 true>'));
  assert.ok(cardOf(gs, 'gate:b-1:choices').html.includes('<choices b-1 true>'));
  assert.deepStrictEqual(gs.filter(g => g.gate).map(g => [g.key, g.gate]), [['waiting:a-1', 'a-1'], ['waiting:b-1', 'b-1']]);
  assert.ok(gs.filter(g => !g.key.startsWith('waiting:')).every(g => !('gate' in g)));
  // The gate the column starts on: the opened card's gate if it waits, else the first that waits, else none.
  assert.strictEqual(call('cardDialogDockGate', all, 'b-1').id, 'b-1');
  assert.strictEqual(call('cardDialogDockGate', all, 'p-1').id, 'a-1');
  assert.strictEqual(call('cardDialogDockGate', all, null).id, 'a-1');
  wait([]);
  assert.strictEqual(call('cardDialogDockGate', all, null), null);
});

test('a task with nothing to judge leaves out what it lacks; a record is all the review has', () => {
  wait([]);
  let gs = groupsOf([]);
  assert.deepStrictEqual(keysOf(gs), ['plan', 'history', 'details']);
  assert.ok(!cardOf(gs, 'details:lastmsg'));
  SESSION.s = { present: true, agentSession: { lastMessage: 'done it' } };
  assert.ok(cardOf(groupsOf([]), 'details:lastmsg').html.includes('done it'));
  // The dialog's card is the expanded one, and the whole text is asked for the session it is about.
  SESSION.s = { id: 'w1', present: true, agentSession: { lastMessage: 'done it' } };
  SESSION.asked.length = 0;
  const html = cardOf(groupsOf([]), 'details:lastmsg').html;
  assert.ok(html.includes('data-expanded="true"') && html.includes('data-for="w1"'), html);
  assert.deepStrictEqual(SESSION.asked.map(x => x.id), ['w1']);
  SESSION.asked.length = 0;
  SESSION.s = { id: 'w1', present: false, agentSession: { lastMessage: 'x' } };
  groupsOf([]);
  assert.deepStrictEqual(SESSION.asked, []);
  SESSION.s = { present: false, agentSession: { lastMessage: 'x' } };
  assert.ok(!cardOf(groupsOf([]), 'details:lastmsg'));
  SESSION.s = null;
  const rec = dgate('r-1', 'diff', '2026-10-01T00:00:00Z', { ...diffGate, id: 'r-1', wait: false, openedAt: '2026-10-01T00:00:00Z' });
  gs = groupsOf([rec]);
  assert.deepStrictEqual(keysOf(gs), ['review', 'diff', 'plan', 'history', 'details']);
  assert.ok(cardOf(gs, 'review:findings') && cardOf(gs, 'review:report') && cardOf(gs, 'review:diff'));
  const kv = cardOf(gs, 'details:kv').html;
  assert.ok(kv.includes('<dt>Issue</dt>') && kv.includes('<dt>PR</dt>') && kv.includes('<dt>置いている</dt>'));
});

test('a button of the panel finds its card by section and gate, then by gate, then (without a gate) by section; else the top', () => {
  const w = dgate('w-1', 'diff', '2026-10-01T00:00:00Z', { ...diffGate, id: 'w-1', waiting: true, options: ['approve'] });
  const gs = groupsOf([w]);
  const at = (section, gate) => call('cardDialogResolveTarget', gs, { section, gate });
  assert.strictEqual(at('facts', 'w-1'), 'gate:w-1:facts');
  assert.strictEqual(at('findings', 'w-1'), 'review:findings');
  assert.strictEqual(at('nothing', 'w-1'), 'gate:w-1:head', 'the first card of that gate');
  assert.strictEqual(at('goal', null), 'plan:goal', 'the section alone');
  assert.strictEqual(at('findings', 'other'), null, 'never another gate\'s card of the same section');
  assert.strictEqual(at('nothing', 'other'), null);
  assert.strictEqual(at(null, null), null);
  assert.strictEqual(call('cardDialogResolveTarget', gs, null), null);
  // The head's button opens at the first gate that waits.
  assert.strictEqual(at(null, 'w-1'), 'gate:w-1:head');
  wait([]);
});

test('the index marks the last group that has reached the top of the scroll', () => {
  const act = (tops, st, off) => call('cardDialogActiveGroup', tops, st, off);
  assert.strictEqual(act([0, 300, 900], 0), 0);
  assert.strictEqual(act([0, 300, 900], 280), 1, 'within the offset');
  assert.strictEqual(act([0, 300, 900], 299 - 24), 0);
  assert.strictEqual(act([0, 300, 900], 5000), 2);
  assert.strictEqual(act([], 10), -1);
});

test('the index and the content are escaped, and carry no answer controls', () => {
  const w = dgate('w-<1>', 'question', '2026-10-01T00:00:00Z', { title: '"<img src=x>"', waiting: true, options: ['answer'] });
  wait([w]);
  const gs = call('cardDialogTaskGroups', task, [w]);
  const h = call('cardDialogTaskHtml', gs);
  assert.ok(!h.index.includes('<img') && !h.content.includes('<img src=x>'));
  assert.ok(!/\sid=/.test(h.index));
  assert.ok(/<button type="button" data-cd-go-group="waiting:w-&lt;1&gt;" class="cd-ix-stack">/.test(h.index));
  assert.ok(/<button type="button" data-cd-go-group="waiting:w-&lt;1&gt;" data-cd-go-card="gate:w-&lt;1&gt;:head">/.test(h.index));
  assert.ok(h.content.includes('data-cd-card="gate:w-&lt;1&gt;:head"'));
  assert.ok(!h.content.includes('cd-dock') && !h.content.includes('<decide'));
  wait([]);
});

test('the answer column escapes the gate, has a switch only when two or more wait, and shows no gate by itself', () => {
  const w = dgate('w-<1>', 'question', '2026-10-01T00:00:00Z', { title: '"<img src=x>"', waiting: true });
  const x = dgate('x-2', 'diff', '2026-10-02T00:00:00Z', { title: 'Other', waiting: true });
  const one = call('cardDialogDockHtml', [w]);
  assert.ok(!one.includes('<img'));
  assert.ok(one.includes('class="cd-dock" data-cd-dock-for="w-&lt;1&gt;" role="group" aria-label="【question】&quot;&lt;img src=x&gt;&quot; に答える" hidden>'));
  assert.ok(one.includes('<button type="button" class="btn-m3-text" data-cd-goto="w-&lt;1&gt;">この gate へ</button>'));
  assert.ok(one.includes('<decide w-<1>>'));
  assert.ok(!one.includes('cd-dock-switch') && !one.includes('data-cd-dock="'));
  const two = call('cardDialogDockHtml', [w, x]);
  assert.ok(two.includes('role="group" aria-label="答える gate"'));
  assert.ok(two.includes('<button type="button" data-cd-dock="w-&lt;1&gt;" aria-pressed="false" title="【question】&quot;&lt;img src=x&gt;&quot;">'));
  assert.ok(two.includes('data-cd-dock="x-2"'));
  // Which dock shows is decided by cardDialogShowDock, not by the markup: the same for every gate.
  assert.strictEqual(two.match(/ hidden>/g).length, 2);
  assert.strictEqual(two.match(/aria-pressed="true"/g), null);
  assert.strictEqual(call('cardDialogDockHtml', []), '');
});

test('the plan shown in the plan group keeps its 決定事項 there and not again in its answered card', () => {
  const p = dgate('p-9', 'plan', '2026-10-01T00:00:00Z', { ...plan, id: 'p-9', openedAt: '2026-10-01T00:00:00Z' });
  const other = dgate('p-8', 'plan', '2026-09-30T00:00:00Z', { ...plan, id: 'p-8', openedAt: '2026-09-30T00:00:00Z', decided: 'older choice' });
  const gs = groupsOf([other, p]);
  assert.ok(cardOf(gs, 'plan:decided').html.includes('chose A'));
  assert.ok(!cardOf(gs, 'gate:p-9').html.includes('chose A'));
  assert.ok(cardOf(gs, 'gate:p-8').html.includes('older choice'));
});

test('an answered gate is one wide block with its own heading, its parts in a grid', () => {
  const p = dgate('p-1', 'plan', '2026-10-01T00:00:00Z', { ...plan, id: 'p-1', title: '<img src=x>', openedAt: '2026-10-01T00:00:00Z' });
  const d = dgate('d-1', 'diff', '2026-10-02T00:00:00Z', { ...diffGate, id: 'd-1', openedAt: '2026-10-02T00:00:00Z', decision: 'changes', comment: '<b>x</b>', answeredAt: 'a' });
  const latest = dgate('d-2', 'diff', '2026-10-03T00:00:00Z', { id: 'd-2', decision: 'approve' });
  const gs = groupsOf([p, d, latest]);
  const pc = cardOf(gs, 'gate:p-1'), dc = cardOf(gs, 'gate:d-1');
  assert.ok(pc.wide && dc.wide);
  for (const c of [pc, dc]) {
    assert.ok(c.html.startsWith('<div class="cd-gate"><div class="cd-gate-head">'));
    assert.ok(c.html.includes('cd-gate-title'));
    assert.ok(!c.html.includes('<status>'), 'the panel-style head is not in this path');
  }
  assert.ok(dc.html.includes('【diff】Diff'));
  assert.ok(dc.html.includes('差し戻した') && dc.html.includes('cd-badge cd-bad') && dc.html.includes('ago(a)に回答'));
  assert.ok(dc.html.includes('&lt;b&gt;x&lt;/b&gt;') && dc.html.includes('止めた理由: r1'));
  assert.ok(!pc.html.includes('<img'));
  assert.ok(pc.html.includes('&lt;img src=x&gt;') && pc.html.includes('承認した') && !pc.html.includes('cd-badge cd-bad'));
  assert.ok(/<div class="cd-part"><div class="panel" data-expand="facts"/.test(dc.html));
  assert.ok(pc.html.includes('<div class="cd-part cd-wide"><choices p-1 false></div>'));
  assert.ok(/<div class="cd-part cd-wide"><div class="panel" data-expand="rounds"/.test(dc.html));
  assert.ok(/<div class="cd-part cd-wide"><div class="panel" data-expand="findings"/.test(dc.html));
  assert.ok(dc.html.includes(call('filesTableHtml', { ...diffGate, id: 'd-1' })) && dc.html.includes(`<div class="cd-part cd-wide">${call('filesTableHtml', { ...diffGate, id: 'd-1' })}</div>`));
  assert.ok(dc.html.includes(`<div class="cd-part cd-wide">${call('diffCardHtml', { ...diffGate, id: 'd-1' })}</div>`));
  assert.ok(!cardOf(gs, 'gate:d-2').html.includes('data-expand="diff"'));
  // A recorded choice with no matching option is still shown, escaped.
  const orphan = groupsOf([dgate('c-1', 'question', '2026-10-01T00:00:00Z', { decision: 'choice', choice: '<z>', choices: [{ id: 'a', label: 'A' }] })]);
  assert.ok(cardOf(orphan, 'gate:c-1').html.includes('選んだ案: &lt;z&gt;'));
  const bare = cardOf(gs, 'gate:d-2');
  for (const cls of ['cd-gate-when', 'cd-gate-answer', 'cd-gate-why', 'cd-gate-parts']) assert.ok(!bare.html.includes(cls), cls);
  wait([]);
});

test('in the dialog the wait head leaves out the focus it carries and the plan head leaves out the chips', () => {
  const g = dgate('w-1', 'diff', '2026-10-01T00:00:00Z', { ...diffGate, id: 'w-1', waiting: true, focus: 'look here' });
  assert.ok(call('gateWaitHeadHtml', g).includes('rv-wait-focus'));
  assert.ok(!call('gateWaitHeadHtml', g, { focus: false }).includes('rv-wait-focus'));
  assert.strictEqual(call('gateWaitHeadHtml', g, { focus: false }).replace(/\s+/g, ''), call('gateWaitHeadHtml', { ...g, focus: '' }).replace(/\s+/g, ''));
  const gs = groupsOf([g]);
  assert.ok(!cardOf(gs, 'gate:w-1:head').html.includes('rv-wait-focus'));
  assert.ok(cardOf(gs, 'gate:w-1:focus').html.includes('look here'));
  const two = [{ ...plan, id: 'p-1' }, { ...plan, id: 'p-2', decision: undefined, answeredAt: undefined }];
  assert.ok(call('planHeadCardHtml', two[0], two).includes('data-pick'));
  const bare = call('planHeadCardHtml', two[0], two, { chips: false });
  assert.ok(!bare.includes('data-pick') && !bare.includes('計画 2件'));
  assert.ok(call('planHeadCardHtml', two[0], two).includes('plan focus'));
  assert.ok(!call('planHeadCardHtml', two[0], two, { focus: false }).includes('plan focus'));
  assert.ok(!cardOf(groupsOf(two), 'plan:head').html.includes('plan focus'));
  assert.ok(!cardOf(groupsOf(two), 'plan:head').html.includes('計画 2件'));
});

test("a waiting plan of another board shows as waiting in that board's state, and not in the page's own", () => {
  const w = dgate('p-w', 'plan', '2026-10-01T00:00:00Z', { ...plan, id: 'p-w', openedAt: '2026-10-01T00:00:00Z', options: ['approve'], choices: [{ id: 'a', label: 'A' }] });
  // The page holds no open gate: the plan waits only in the board's state the cards are drawn under.
  wait([]);
  const plain = html => JSON.parse(JSON.stringify(html));
  const planHead = gs => cardOf(gs, 'plan:head')?.html || '';
  const page = plain(call('cardDialogTaskGroups', task, [w]));
  assert.ok(!planHead(page).includes('承認待ち'));
  const scoped = plain(ctx.withViewState({ gates: [w], tasks: [task] }, () => call('cardDialogTaskGroups', task, [w])));
  assert.ok(planHead(scoped).includes('承認待ち'), 'the plan card says it waits for approval');
  // The page's state is back as it was once the cards are drawn.
  assert.ok(!planHead(plain(call('cardDialogTaskGroups', task, [w]))).includes('承認待ち'));
});

/* The plan's steps and what was found, which a gate carries beside the frames. */
const stepped = { ...plan, id: 'p-s', decision: undefined, answeredAt: undefined,
  steps: [{ text: 'first <b>', detail: 'src/a.rs' }, { text: 'second' }],
  sections: [{ heading: '原因 <x>', body: 'because <i>' }, { heading: '対応策', body: 'fix it', stage: 'plan' }] };

test('stepsCardHtml is an ordered list, escaped, with the detail as a line under its step; empty gives nothing', () => {
  const h = call('stepsCardHtml', stepped);
  assert.ok(h.includes('data-expand="steps"') && h.includes('計画の手順') && h.includes('<ol class="steps">'));
  assert.ok(h.indexOf('first &lt;b&gt;') < h.indexOf('second'));
  assert.ok(!h.includes('<b>') && h.includes('<div class="step-detail">src/a.rs</div>'));
  assert.strictEqual((h.match(/<li>/g) || []).length, 2);
  assert.strictEqual(call('stepsCardHtml', plan), '');
  assert.strictEqual(call('stepsCardHtml', { ...plan, steps: [] }), '');
});

test('sectionsCardsHtml draws the found blocks, then the plan ones in a panel of their own, headings escaped and bodies as markdown', () => {
  const h = call('sectionsCardsHtml', stepped);
  assert.ok(h.includes('調べたこと') && h.includes('<h4 class="sec-heading">原因 &lt;x&gt;</h4>'));
  assert.ok(!h.includes('<x>') && h.includes('<p>because &lt;i></p>'));
  assert.ok(h.indexOf('原因') < h.indexOf('対応策') && h.indexOf('調べたこと') < h.indexOf('>計画<'));
  assert.strictEqual(call('sectionsCardsHtml', stepped, 'found') + call('sectionsCardsHtml', stepped, 'plan'), h);
  assert.ok(!call('sectionsCardsHtml', stepped, 'found').includes('対応策'));
  assert.ok(!call('sectionsCardsHtml', stepped, 'plan').includes('原因'));
  // A stage this page does not know reads as found.
  assert.ok(call('sectionsCardsHtml', { sections: [{ heading: 'h', body: 'b', stage: 'later' }] }, 'found').includes('>h<'));
  assert.strictEqual(call('sectionsCardsHtml', plan), '');
  assert.strictEqual(call('sectionsCardsHtml', { sections: [] }), '');
});

test('planGateCardsHtml has the sections and the steps after the facts and before the report', () => {
  const h = call('planGateCardsHtml', stepped);
  const at = s => h.indexOf(s);
  assert.ok(at('data-expand="facts"') < at('調べたこと') && at('調べたこと') < at('計画の手順'));
  assert.ok(at('計画の手順') < at('>計画<') && at('>計画<') < at('data-expand="report"'));
});

test('a gate with no tab of its own shows its sections after the frames; the review and check tabs do too', () => {
  const result = { id: 'r-1', kind: 'result', title: 'R', focus: 'ok?', openedAt: 'o', sections: stepped.sections };
  const h = call('gateDetailHtml', result, [result]);
  assert.ok(h.includes(call('framesHtml', result) + call('sectionsCardsHtml', result)));
  const withSteps = call('gateDetailHtml', { ...result, steps: stepped.steps }, [result]);
  assert.ok(withSteps.includes('計画の手順'));
  const d = { ...diffGate, sections: stepped.sections };
  wait([d]);
  assert.ok(call('reviewTab', task, [d], {}).includes(call('framesHtml', d) + call('sectionsCardsHtml', d)));
  const v = { ...verifyGate, sections: stepped.sections };
  wait([v]);
  assert.ok(call('checkTab', task, [v], {}).includes(call('framesHtml', v) + call('sectionsCardsHtml', v)));
  wait([]);
});

test('the dialog has a card for the steps of the plan and of another kind, and one for the sections of a waiting gate', () => {
  const p = dgate('p-s', 'plan', '2026-10-01T00:00:00Z', { ...stepped, id: 'p-s', openedAt: '2026-10-01T00:00:00Z', decision: 'approve', answeredAt: 'a' });
  const r = dgate('r-1', 'result', '2026-10-02T00:00:00Z', { title: 'R', focus: 'ok?', waiting: true, options: ['ack'], sections: stepped.sections, steps: stepped.steps });
  const gs = groupsOf([p, r]);
  assert.ok(cardOf(gs, 'plan:steps').html.includes('first &lt;b&gt;'));
  assert.strictEqual(gs.find(g => g.key === 'plan').cards.map(c => c.key).indexOf('plan:steps'),
    gs.find(g => g.key === 'plan').cards.map(c => c.key).indexOf('plan:head') + 1);
  assert.ok(cardOf(gs, 'gate:r-1:sections').html.includes('調べたこと') && cardOf(gs, 'gate:r-1:sections').wide);
  assert.ok(cardOf(gs, 'gate:r-1:steps').html.includes('計画の手順'));
  // Not drawn twice: the shown plan keeps its steps in the plan group.
  assert.ok(!cardOf(gs, 'gate:p-s').html.includes('data-expand="steps"'));
  assert.ok(cardOf(gs, 'gate:p-s').html.includes('data-expand="sections"'));
  assert.strictEqual(call('cardDialogResolveTarget', gs, { section: 'steps', gate: 'p-s' }), 'plan:steps');
  assert.strictEqual(call('cardDialogResolveTarget', gs, { section: 'sections', gate: 'r-1' }), 'gate:r-1:sections');
  // A waiting plan has no steps card of its own in its group.
  const w = dgate('p-w', 'plan', '2026-10-03T00:00:00Z', { ...stepped, id: 'p-w', waiting: true, options: ['approve'], openedAt: '2026-10-03T00:00:00Z' });
  assert.ok(!cardOf(groupsOf([w]), 'gate:p-w:steps'));
  wait([]);
});

test("filesOf reads paths Git quotes and keeps each file's counts its own", () => {
  const plain = x => JSON.parse(JSON.stringify(x));
  const diffBoth = [
    'diff --git a/a.rs b/a.rs',
    '--- a/a.rs',
    '+++ b/a.rs',
    '@@ -1 +1 @@',
    '-x',
    '+y',
    'diff --git "a/docs/\\346\\227\\245.md" "b/docs/\\346\\227\\245.md"',
    '--- "a/docs/\\346\\227\\245.md"',
    '+++ "b/docs/\\346\\227\\245.md"',
    '@@ -1 +1,2 @@',
    '-x',
    '+y',
    '+z',
  ].join('\n');
  assert.deepStrictEqual(plain(call('filesOf', diffBoth)), [
    { path: 'a.rs', add: 1, del: 1 },
    { path: 'docs/日.md', add: 2, del: 1 },
  ]);

  const diffQuoted = [
    'diff --git "a/docs/\\346\\227\\245.md" "b/docs/\\346\\227\\245.md"',
    '--- "a/docs/\\346\\227\\245.md"',
    '+++ "b/docs/\\346\\227\\245.md"',
    '@@ -1 +1,2 @@',
    '-x',
    '+y',
    '+z',
  ].join('\n');
  const quoted = plain(call('filesOf', diffQuoted));
  assert.strictEqual(quoted.length, 1);
  assert.deepStrictEqual(quoted, [{ path: 'docs/日.md', add: 2, del: 1 }]);

  assert.deepStrictEqual(plain(call('filesOf', '')), []);
  assert.deepStrictEqual(plain(call('filesOf', undefined)), []);

  const diffDashes = [
    'diff --git a/a.rs b/a.rs',
    '--- a/a.rs',
    '+++ b/a.rs',
    '@@ -1 +1 @@',
    '--- old',
    '+new',
  ].join('\n');
  assert.deepStrictEqual(plain(call('filesOf', diffDashes)), [
    { path: 'a.rs', add: 1, del: 1 },
  ]);
});

/* The buttons of a result gate. */
const dockCtx = vm.createContext({ gateRef: g => g.id, ideTitle: () => '', parkButtonHtml: () => '', roundsHintHtml: () => '' });
vm.runInContext([
  cut(utilSrc, /^const esc = [^\n]*\n/m),
  cut(utilSrc, /^const DECISION_LABEL = [^\n]*\nconst DECISION_LABEL_OF_KIND = \{[\s\S]*?\n\};\nconst decisionLabel = [^\n]*\n[^\n]*\n/m),
  fn(decSrc, 'gateDockHtml'),
].join('\n'), dockCtx);

test('a result gate offering ack, ask and task has three buttons with the result wording', () => {
  const g = { id: 'r-1', kind: 'result', options: ['ack', 'ask', 'task'] };
  const h = vm.runInContext('gateDockHtml(__g)', Object.assign(dockCtx, { __g: g }));
  const acts = [...h.matchAll(/data-act="([a-z]+)"/g)].map(m => m[1]).filter(a => !['talk', 'close'].includes(a));
  assert.deepStrictEqual(acts, ['ack', 'ask', 'task']);
  for (const label of ['分かった', '続けて聞く', 'これをタスクにする']) assert.ok(h.includes(`<span>${label}</span>`), label);
  assert.ok(h.includes('add_task'));
  // A gate stored with the old two options keeps them.
  Object.assign(dockCtx, { __g: { ...g, options: ['ack', 'ask'] } });
  assert.ok(!vm.runInContext('gateDockHtml(__g)', dockCtx).includes('data-act="task"'));
});

test('the dialog reads the plan from the task too: a plan with no gate, and a plan that differs from an older gate', () => {
  const planGroup = (t, all) => {
    wait(all.filter(g => g.id === 'p-2'));
    const g = JSON.parse(JSON.stringify(call('cardDialogTaskGroups', t, all))).find(x => x.key === 'plan');
    wait([]);
    return g;
  };
  const text = g => g.cards.map(c => c.html).join('');
  // Only task.plan: no 「計画はまだ出ていない」, the plan card with its mark, problem and goal once.
  const byHand = planGroup({ ...task, plan: { ...ownPlan, gate: undefined } }, []);
  const hand = text(byHand);
  assert.ok(!hand.includes('計画はまだ出ていない'));
  assert.ok(hand.includes('data-task-plan="nogate"') && hand.includes('first &lt;b&gt;'));
  assert.strictEqual(count(hand, 'own &lt;problem>'), 1);
  assert.strictEqual(count(hand, 'own goal'), 1);
  assert.ok(!byHand.cards.some(c => c.key === 'plan:head'));
  // task.plan differs from an older gate: the plan is the task's, the gate stays as what was asked.
  const older = { ...plan, steps: [{ text: 'older step' }] };
  const both = planGroup({ ...task, plan: { ...ownPlan, gate: 'p-9' } }, [older]);
  const h = text(both);
  assert.strictEqual(count(h, 'own goal'), 1);
  assert.ok(!h.includes('<p>prob</p>') && !h.includes('<p>goal</p>'));
  assert.ok(h.includes('older step') && h.includes('first &lt;b&gt;'));
  assert.ok(both.cards.some(c => c.key === 'plan:head' && c.label === '計画のゲート'));
  // The gate that wrote the plan does not repeat the steps.
  const same = planGroup({ ...task, plan: { ...ownPlan, gate: 'p-1' } }, [{ ...plan, steps: [{ text: 'gate step' }] }]);
  assert.ok(!text(same).includes('gate step'));
});

test('a finished task whose approvedPlan the board left off is still 承認済み from the approved gate in its history', () => {
  const done = { ...task, status: 'done', plan: ownPlan };
  assert.strictEqual(mark(done, [plan]), 'approved');
  assert.strictEqual(mark(done, [{ ...plan, decision: 'choice' }]), 'approved');
  assert.strictEqual(mark(done, [{ ...plan, decision: 'changes' }]), 'unapproved');
});

test('the task detail marks the task plan by planApprovalOf, never by an unrelated open plan gate', () => {
  load([fn(read('task-detail-view.js'), 'detailPlanOf'), cst(read('task-detail.js'), 'tdMuted'), fn(read('task-detail.js'), 'tdPlanHtml')].join('\n'));
  const waitingOld = { ...plan, id: 'p-old', decision: undefined, answeredAt: undefined };
  const t = { ...task, plan: { ...ownPlan, gate: undefined } };
  wait([waitingOld]);
  const shown = call('detailPlanOf', t, waitingOld, [waitingOld]);
  assert.strictEqual(call('isWaiting', shown), false);
  const html = call('tdPlanHtml', shown, call('isWaiting', shown));
  assert.ok(html.includes('ゲートなしで書いた計画') && !html.includes('承認待ち'));
  // Its content is the task's, not the gate's.
  assert.strictEqual(shown.goal, 'own goal');
  // A finished task's approved gate in history: 承認済み.
  wait([]);
  const done = { ...task, status: 'done', plan: ownPlan };
  assert.ok(call('tdPlanHtml', call('detailPlanOf', done, plan, [plan]), false).includes('承認済み'));
  // No task plan: the gate as it was.
  assert.strictEqual(call('detailPlanOf', task, plan, [plan]), plan);
});

test('the request card follows the plan the overview picked: a steps-only task plan is no problem even beside an older gate that has one', () => {
  wait([]);
  const stepsOnly = { ...task, plan: { steps: [{ text: 'implement' }], revision: 1, updatedAt: 'u' } };
  assert.ok(!call('planProblemShown', stepsOnly, [plan], {}));
  const h = call('overviewTab', stepsOnly, [plan], {}, {});
  assert.ok(h.includes('thing') && !h.includes('<p>prob</p>'));
  assert.strictEqual(call('planProblemShown', { ...task, plan: ownPlan }, [plan], {}), 'own <problem>');
  assert.strictEqual(call('planProblemShown', task, [plan], {}), 'prob');
});
