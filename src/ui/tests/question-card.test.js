/* The pure half of the question card (src/ui/question-card.js), run with `node --test`. The page's scripts share one global scope, so
   they are loaded into one context the way the browser does; the buttons and the choices are cut out of decide.js, which needs the
   page. */
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

const ctx = vm.createContext({ TextEncoder, TextDecoder });
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'aide-display.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(read('question-card.js'), 'question-card.js');
const { questionCardHtml, questionCardArticleHtml, quickRowHtml, questionStripHtml, questionAskHtml, questionEarlierHtml, QUESTION_ASK_OF_KIND } = vm.runInContext(
  '({ questionCardHtml, questionCardArticleHtml, quickRowHtml, questionStripHtml, questionAskHtml, questionEarlierHtml, QUESTION_ASK_OF_KIND })', ctx);

const DIFF = [
  'diff --git a/src/a.rs b/src/a.rs', 'index 1..2 100644', '--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,2 +1,3 @@ fn a',
  ' fn a() {', '-    old();', '+    new();', '+    more();', ' }',
].join('\n');
const task = { id: 't1', title: 'Add a timer', body: 'Make the timer stop at zero', issueUrl: 'https://github.com/acme/widget/issues/7', issueSnapshot: { title: 'Timer', body: 'Details' } };
const planGate = {
  id: 'p1', kind: 'plan', title: 'Plan title', openedAt: '20260922T041233Z', wait: true, options: ['approve', 'changes'],
  problem: 'The timer never stops', goal: 'Stop at zero',
  sections: [{ heading: 'Where it loops', body: 'in tick()', stage: 'found' }, { heading: 'Rollout', body: 'one step', stage: 'plan' }],
  steps: [{ text: 'Fix tick', detail: 'guard zero' }, { text: 'Add test' }],
  facts: ['3 files read'], unsure: 'Not sure about rounding', decided: 'Use integers',
  focus: 'Is **quokka** time fine?',
  choices: [{ id: 'a', label: 'Integers', recommended: true, points: [] }, { id: 'b', label: 'Floats' }],
};
const card = (g, c = {}) => questionCardHtml(g, { task, plan: g.kind === 'plan' ? g : null, nwo: 'acme/widget', issue: { number: 7, url: task.issueUrl, hasBody: true }, key: `s/${g.id}`, ...c });
const all = c => c.head + c.body + c.foot;
const count = (h, needle) => h.split(needle).length - 1;

test('a plan card reads in order: request, found, plan, the question, the choices; the question appears once', () => {
  const c = card(planGate);
  const at = s => { const i = c.body.indexOf(s); assert.ok(i >= 0, s); return i; };
  assert.ok(at('あなたの依頼') < at('調べたこと'));
  assert.ok(at('調べたこと') < at('計画'));
  assert.ok(at('計画') < at('qc-ask'));
  assert.ok(at('qc-ask') < at('data-choice'));
  assert.equal(count(all(c), 'quokka'), 1);
  assert.ok(c.head.includes('Add a timer'));
  assert.ok(c.body.includes('Stop at zero'));
  assert.ok(c.body.includes('懸念点') && c.body.includes('決めたこと'));
  assert.ok(c.body.indexOf('Fix tick') < c.body.indexOf('懸念点'));
  assert.equal(count(c.body, 'data-choice='), 2);
});

test('a question with nothing around it draws no empty heading', () => {
  const c = questionCardHtml({ id: 'q', kind: 'question', title: 'Which one?', focus: 'Pick a color', wait: true, options: ['answer'] }, { key: 'q' });
  for (const h of ['あなたの依頼', '調べたこと', '計画', '懸念点', '前に決まっていること']) assert.ok(!c.body.includes(h), h);
  assert.ok(c.body.includes('Pick a color'));
  assert.ok(c.body.includes('qc-ask'));
});

test('a plan gate from before steps and sections: decided is the plan, shown once; the problem is under 調べたこと', () => {
  const g = { id: 'o', kind: 'plan', title: 'Old', wait: true, options: ['approve'], decided: 'step one then step two', problem: 'the bug', focus: 'ok?' };
  const c = card(g);
  assert.equal(count(c.body, 'step one then step two'), 1);
  assert.ok(!c.body.includes('決めたこと'));
  assert.ok(c.body.indexOf('調べたこと') < c.body.indexOf('the bug'));
  assert.ok(c.body.indexOf('the bug') < c.body.indexOf('計画'));
  assert.ok(c.body.indexOf('計画') < c.body.indexOf('step one'));
});

test('a diff card is 計画とやったこと, marks the approved steps done and lists what is open and declined', () => {
  const approved = { ...planGate, choice: 'a', comment: 'go on', answeredAt: '20260922T050000Z', wait: false };
  const g = {
    id: 'd1', kind: 'diff', title: 'Diff', wait: true, options: ['approve', 'changes'], diff: DIFF, focus: 'Look at tick', reviewRounds: [{}, {}],
    findings: [{ severity: 'must', outcome: 'open', text: 'open one', location: 'src/a.rs:2' }, { severity: 'want', outcome: 'declined', text: 'declined one', reason: 'by design' }, { severity: 'want', outcome: 'fixed', text: 'fixed one' }],
  };
  const c = card(g, { task: { ...task, approvedPlan: approved }, plan: approved });
  assert.ok(c.body.includes('計画とやったこと'));
  assert.ok(c.body.includes('計画は承認済み'));
  assert.equal(count(c.body, '<span class="qc-chip ok">済</span>'), 2);
  assert.ok(c.body.includes('残っている指摘 1件') && c.body.includes('open one'));
  assert.ok(c.body.includes('見送った指摘 1件') && c.body.includes('by design'));
  assert.ok(c.body.includes('+2 −1') && c.body.includes('1 ファイル') && c.body.includes('セルフレビュー 2 巡') && c.body.includes('修正済み 1'));
  assert.ok(c.foot.includes('差分を全部見る'));
  assert.ok(c.body.includes('前に決まっていること') && c.body.includes('計画で『Integers』を選んだ — go on'));
  assert.ok(c.body.indexOf('前に決まっていること') < c.body.indexOf('qc-ask'));
});

test('a diff card whose approved plan is from before steps shows that plan\'s decided as the plan', () => {
  const approved = { id: 'op', kind: 'plan', title: 'Old', decided: 'step one then step two', wait: false };
  const g = { id: 'd2', kind: 'diff', title: 'Diff', wait: true, options: ['approve'], diff: DIFF, focus: 'ok?' };
  const c = card(g, { task: { ...task, approvedPlan: approved }, plan: approved });
  assert.ok(c.body.includes('計画は承認済み'));
  assert.equal(count(c.body, 'step one then step two'), 1);
  assert.ok(c.body.indexOf('step one then step two') < c.body.indexOf('qc-ask'));
});

test('steps are marked done only where the worker says so once one does; 前に決まっていること needs a choice or a comment', () => {
  const approved = { ...planGate, steps: [{ text: 'a', done: true }, { text: 'b', done: false }], wait: false };
  const g = { id: 'd2', kind: 'diff', title: 'D', wait: true, options: [], focus: 'f' };
  const c = card(g, { task: { ...task, approvedPlan: approved }, plan: approved });
  assert.equal(count(c.body, '<span class="qc-chip ok">済</span>'), 1);
  assert.equal(questionEarlierHtml(g, { task: { approvedPlan: { decision: 'approve' } } }), '');
  assert.ok(questionEarlierHtml(g, { task: { approvedPlan: { comment: 'fine' } } }).includes('計画を承認した — fine'));
  assert.equal(questionEarlierHtml({ ...g, kind: 'result' }, { task: { approvedPlan: { comment: 'fine' } } }), '');
});

test('a verify card shows the commands, the failed output, the manual checks and how to run it', () => {
  const g = {
    id: 'v1', kind: 'verify', title: 'V', wait: true, options: ['approve', 'changes'], worktree: '/w/x', focus: 'Try it',
    commands: [{ command: 'make check', result: 'pass', attempts: 2 }, { command: 'cargo test', result: 'fail', output: 'boom' }],
    manual: ['open the page'], run: 'adj serve',
  };
  const c = card(g);
  assert.ok(c.body.includes('2回目で通過'));
  assert.ok(c.body.includes('<details open') && c.body.includes('boom'));
  assert.ok(c.body.includes('data-manual-gate="v1"') && c.body.includes('data-manual-index="0"'));
  assert.ok(c.body.includes('動かし方') && c.body.includes('adj serve'));
  assert.ok(c.foot.includes('data-qc-ide'));
  assert.ok(c.body.includes('計画とやったこと'));
});

test('a result puts found sections under 調べたこと, plan sections under 計画, and a long body behind a link', () => {
  const g = {
    id: 'r1', kind: 'result', title: 'R', wait: true, options: ['ack', 'ask', 'task'], focus: 'Is that what you wanted?', body: 'the whole report',
    sections: [{ heading: 'Found A', body: 'aaa' }, { heading: 'Next B', body: 'bbb', stage: 'plan' }],
  };
  const c = card(g);
  assert.ok(c.body.indexOf('調べたこと') < c.body.indexOf('Found A') && c.body.indexOf('Found A') < c.body.indexOf('計画'));
  assert.ok(c.body.indexOf('計画') < c.body.indexOf('Next B'));
  assert.ok(c.body.includes('data-qc-open="report"') && !c.body.includes('the whole report'));
  for (const label of ['分かった', '続けて聞く', 'これをタスクにする']) assert.ok(c.foot.includes(label), label);
  // With no sections the report is the found part itself.
  const plain = card({ ...g, sections: [] });
  assert.ok(plain.body.indexOf('調べたこと') < plain.body.indexOf('the whole report'));
  assert.ok(!plain.body.includes('data-qc-open="report"'));
});

test('a missing focus falls back to the title when a task names the card, else to the question of the kind', () => {
  assert.equal(questionAskHtml({ kind: 'plan', title: 'Plan title' }, { task }), 'Plan title');
  assert.equal(questionAskHtml({ kind: 'plan', title: 'Plan title' }, {}), QUESTION_ASK_OF_KIND.plan);
  assert.equal(questionAskHtml({ kind: 'nope' }, {}), QUESTION_ASK_OF_KIND.question);
  for (const [kind, text] of Object.entries(QUESTION_ASK_OF_KIND)) assert.equal(questionAskHtml({ kind }, {}), text);
});

test('a quick row has the question, chips and choice buttons, the recommended one filled, and no comment box', () => {
  const g = { id: 'k', kind: 'question', quick: true, title: 'T', focus: 'Use **UTC**?', facts: ['one fact'], choices: [{ id: 'y', label: 'Yes' }, { id: 'n', label: 'No', recommended: true }] };
  const h = quickRowHtml(g, { key: 's/k' });
  assert.ok(h.includes('data-qc-gate="s/k"') && h.includes('Use UTC?') && h.includes('qc-chip'));
  assert.ok(/class="pick btn-m3-primary" data-choice="n"/.test(h) && /class="pick btn-m3-tonal" data-choice="y"/.test(h));
  assert.ok(!h.includes('<textarea'));
  assert.ok(/class="pick btn-m3-primary" data-choice="y"/.test(quickRowHtml({ ...g, choices: [{ id: 'y', label: 'Yes' }, { id: 'n', label: 'No' }] })));
  assert.ok(quickRowHtml({ ...g, choices: [], options: ['ack'] }).includes('data-act="ack"'));
});

test('what comes from a gate is escaped', () => {
  const evil = '<script>alert(1)</script>';
  const g = { ...planGate, title: evil, focus: undefined, facts: [evil], sections: [{ heading: evil, body: 'x' }], choices: [{ id: 'a', label: evil }] };
  const c = card(g, { task: { ...task, title: evil } });
  assert.ok(!all(c).includes('<script>'));
  assert.ok(!quickRowHtml({ ...g, quick: true }, {}).includes('<script>'));
  assert.ok(!questionStripHtml([g], {}).includes('<script>'));
});

test('the card has no generic task-detail button, no choices panel heading and no data-action', () => {
  for (const g of [planGate, { id: 'd', kind: 'diff', title: 'D', wait: true, options: ['approve'], diff: DIFF }]) {
    const h = all(card(g)) + questionCardArticleHtml(g, { task, key: 'k' });
    assert.ok(!h.includes('タスク詳細'));
    assert.ok(!h.includes('AI からの提案・選択肢'));
    assert.ok(!h.includes('data-action'));
    assert.ok(!h.includes('expand'));
  }
});

test('the strip has a line that opens the card for a heavy gate and the row itself for a quick one', () => {
  const quick = { id: 'k', kind: 'question', quick: true, title: 'Q', focus: 'Quick?', choices: [{ id: 'y', label: 'Yes' }] };
  const h = questionStripHtml([planGate, quick], g => ({ key: `s/${g.id}` }));
  assert.ok(h.includes('開いて答える') && h.includes('data-qc-open-gate="s/p1"'));
  assert.ok(h.includes('class="qc-quick" data-qc-gate="s/k"'));
  assert.equal(count(h, 'data-qc-open-gate='), 1);
  assert.equal(questionStripHtml([], {}), '');
});
