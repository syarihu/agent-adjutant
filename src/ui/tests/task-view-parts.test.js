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
  sessionOfTask: () => SESSION.s, lastMessageHtml: a => `<div class="tp-lastmsg">${a.lastMessage}</div>`,
  histories: {}, historyKey: t => t.id,
});
const SESSION = { s: null };
const load = (src) => vm.runInContext(src, ctx);
load([
  cut(utilSrc, /^const esc = [^\n]*\n/m),
  cut(utilSrc, /^const httpUrl = [^\n]*\n/m),
  cut(utilSrc, /^const kindOf = [^\n]*\n/m),
  cut(dlgSrc, /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  'const gateRef = g => g._slug ? `${g._slug}/${g.id}` : g.id;',
  'const DECISION = { approve: "承認した", changes: "差し戻した" };',
  'const renderDiff = d => esc(d);',
  cst(tvSrc, 'diffPending'),
  cut(tvSrc, /^const DIFF_LOADING = [^\n]*\n/m),
  cst(tvSrc, 'isWaiting'),
  'const isGithubIssue = u => /issues\\/\\d+/.test(u || "");',
  'const issueNumberOf = u => /(\\d+)$/.exec(u)?.[1] || "";',
  fn(tvSrc, 'gateShownIn'), fn(tvSrc, 'gateForTab'), fn(tvSrc, 'filesOf'),
  'const KIND_OF_TAB = { overview: "plan", review: "diff", check: "verify" };',
  fn(tvSrc, 'actHtml'),
  fn(tvSrc, 'gateHeadHtml'), fn(tvSrc, 'gateStatusHtml'),
  ...['framesHtml', 'factsCardHtml', 'focusCardHtml', 'unsureCardHtml', 'reportCardHtml', 'filesTableHtml',
    'diffCardHtml', 'planSourceOf', 'problemCardHtml', 'issueBodyCardHtml', 'goalCardHtml',
    'instructionCardHtml', 'planHeadCardHtml', 'planGateCardsHtml', 'detailsKvHtml', 'overviewTab',
    'reviewTab', 'checkTab'].map(n => fn(tvSrc, n)),
  cut(decSrc, /^const OUTCOME = [^\n]*\n/m),
  ...['cardDialogWaiting'].map(n => cst(dlgSrc, n)),
  ...['cardDialogAnsweredHeadHtml', 'cardDialogTaskGroups', 'cardDialogResolveTarget', 'cardDialogDockGate', 'cardDialogTaskHtml', 'cardDialogDockHtml', 'cardDialogActiveGroup']
    .map(n => cut(dlgSrc, new RegExp(`^function ${n}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'))),
  ...['gateWaitHeadHtml', 'gateJudgeHtml', 'reviewPanels', 'roundsCardHtml', 'findingsCardHtml',
    'checkPanels', 'commandsCardHtml', 'manualCardHtml'].map(n => fn(decSrc, n)),
].join('\n'));
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
  assert.ok(/<button type="button" data-cd-go-group="waiting:w-&lt;1&gt;">/.test(h.index));
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
