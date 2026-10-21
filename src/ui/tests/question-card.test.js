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
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(read('question-card.js'), 'question-card.js');
const { questionCardHtml, questionCardArticleHtml, quickRowHtml, questionStripHtml, questionAskHtml, questionEarlierHtml, QUESTION_ASK_OF_KIND, qcAnswerButtons, qcNeedsComment, qcQuickIcon } = vm.runInContext(
  '({ questionCardHtml, questionCardArticleHtml, quickRowHtml, questionStripHtml, questionAskHtml, questionEarlierHtml, QUESTION_ASK_OF_KIND, qcAnswerButtons, qcNeedsComment, qcQuickIcon })', ctx);

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
  assert.ok(at('qc-ask') < at('qc-choice'));
  assert.equal(count(all(c), 'quokka'), 1);
  assert.ok(c.head.includes('Add a timer'));
  assert.ok(c.body.includes('Stop at zero'));
  assert.ok(c.body.includes('懸念点') && c.body.includes('決めたこと'));
  assert.ok(c.body.indexOf('Fix tick') < c.body.indexOf('懸念点'));
  assert.equal(c.body.match(/class="qc-choice( rec)?"/g).length, 2);
  assert.equal(count(c.foot, 'data-choice='), 2);
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

test('a diff card reads the approved plan\'s sections; a quick row keeps the whole question', () => {
  const approved = { id: 'np', kind: 'plan', title: 'P', steps: [{ text: 's' }], sections: [{ heading: 'FoundH', body: 'f' }, { heading: 'PlanH', body: 'p', stage: 'plan' }], wait: false };
  const g = { id: 'd3', kind: 'diff', title: 'Diff', wait: true, options: ['approve'], diff: DIFF, focus: 'ok?' };
  const c = card(g, { task: { ...task, approvedPlan: approved }, plan: approved });
  assert.ok(c.body.indexOf('FoundH') > c.body.indexOf('調べたこと') && c.body.indexOf('FoundH') < c.body.indexOf('計画とやったこと'));
  assert.ok(c.body.indexOf('PlanH') > c.body.indexOf('計画とやったこと'));
  const long = 'first line\n' + 'x'.repeat(150) + ' tail';
  const q = quickRowHtml({ id: 'q9', kind: 'question', quick: true, wait: true, focus: long, choices: [{ id: 'a', label: 'A' }] });
  assert.ok(q.includes('first line') && q.includes(' tail'));
});

test('a diff gate\'s own steps are shown as it marks them, over the approved plan\'s', () => {
  const approved = { id: 'ap', kind: 'plan', title: 'P', steps: [{ text: 'old step' }], wait: false };
  const g = { id: 'd4', kind: 'diff', title: 'Diff', wait: true, options: ['approve'], diff: DIFF, focus: 'ok?', steps: [{ text: 'A', done: true }, { text: 'B', done: false }] };
  const c = card(g, { task: { ...task, approvedPlan: approved }, plan: approved });
  assert.ok(c.body.includes('>A<') || c.body.includes('A<span'));
  assert.ok(!c.body.includes('old step') && !c.body.includes('計画は承認済み'));
  assert.equal(count(c.body, '<span class="qc-chip ok">済</span>'), 1);
});

test('a diff with a path Git quotes is counted', () => {
  const quoted = 'diff --git "a/docs/\\346\\227\\245.md" "b/docs/\\346\\227\\245.md"\n--- "a/docs/\\346\\227\\245.md"\n+++ "b/docs/\\346\\227\\245.md"\n@@ -1 +1 @@\n-old\n+new\n';
  const g = { id: 'd5', kind: 'diff', title: 'Diff', wait: true, options: ['approve'], diff: quoted, focus: 'ok?' };
  const c = card(g, { task });
  assert.ok(c.body.includes('1 ファイル') && c.body.includes('+1 −1'));
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

test('a read-only card has the choices with nothing to pick, and no foot', () => {
  const g = { ...planGate, wait: false, decision: 'choice', choice: 'a', answeredAt: '20260922T051233Z' };
  const c = card(g, { readOnly: true });
  assert.equal(c.foot, '');
  // It waits no more: the head says when it was answered.
  assert.ok(c.head.includes('に回答') && !c.head.includes('から待ち'));
  assert.ok(c.body.includes('Integers') && c.body.includes('Floats'));
  assert.equal(count(c.body, 'data-choice='), 0);
  assert.equal(count(c.body, 'data-gate='), 0);
  assert.ok(c.body.includes('選ばれた案'));
  // The same gate not read-only still has both, as buttons in the foot.
  const live = card(planGate);
  assert.equal(count(live.body, 'data-choice='), 0);
  assert.equal(count(live.foot, 'data-choice='), 2);
  assert.ok(live.foot.includes('gate-comment'));
});

test('a plan with choices: one button to a choice in the foot, the recommended first and filled; the choices are cards with no button', () => {
  const c = card(planGate);
  assert.ok(!c.body.includes('pick'));
  assert.ok(c.body.includes('qc-choice rec') && c.body.includes('>推し<') && !c.body.includes('AI の推し案'));
  const a = c.foot.indexOf('data-choice="a"');
  const b = c.foot.indexOf('data-choice="b"');
  assert.ok(a >= 0 && a < b);
  assert.ok(/class="pick btn-m3-primary" data-choice="a"/.test(c.foot));
  assert.ok(/class="pick qc-btn-outline" data-choice="b"/.test(c.foot));
  assert.ok(c.foot.includes('title="Integers"') && !c.foot.includes('aria-label="Integers"') && c.foot.includes('title="Floats"'));
  assert.ok(c.foot.includes('計画を直してほしい') && c.foot.includes('data-act="changes"'));
  assert.ok(!c.foot.includes('見送る') && !c.foot.includes('data-act="reject"') && !c.foot.includes('承認する'));
  const more = c.foot.slice(c.foot.indexOf('<details class="qc-more">'));
  assert.ok(more.includes('data-act="close"') && more.includes('qc-more-menu') && more.includes('aria-label="ほかの操作"'));
  assert.ok(c.foot.indexOf('qc-spacer') < c.foot.indexOf('data-act="talk"') && c.foot.indexOf('data-act="talk"') < c.foot.indexOf('qc-more'));
  // The recommended choice is first even when the worker listed it later; with none recommended none is filled.
  const later = qcAnswerButtons({ kind: 'plan', options: [], choices: [{ id: 'x', label: 'X' }, { id: 'y', label: 'Y', recommended: true }] });
  assert.ok(later[0].lead && later[0].html.includes('data-choice="y"') && !later[1].lead);
  const none = qcAnswerButtons({ kind: 'plan', options: [], choices: [{ id: 'x', label: 'X' }, { id: 'y', label: 'Y' }] });
  assert.ok(none.every(x => !x.lead && !x.html.includes('btn-m3-primary')));
});

test('a choice button says its label when short, its leading id and a verb when long, else the label cut short; the title keeps all', () => {
  const html = (kind, label) => qcAnswerButtons({ kind, options: [], choices: [{ id: 'c', label }] })[0].html;
  assert.ok(html('plan', 'Short label').includes('<span>Short label</span>'));
  const long = 'A. a label that is much longer than sixteen characters';
  assert.ok(html('plan', long).includes('<span>A で進める</span>') && html('plan', long).includes(`title="${long}"`));
  assert.ok(html('diff', long).includes('<span>A で PR に</span>'));
  assert.ok(html('plan', 'B) ' + 'x'.repeat(30)).includes('<span>B で進める</span>'));
  const bare = 'This label has no leading id and is long';
  assert.ok(html('plan', bare).includes(`<span>${[...bare].slice(0, 15).join('')}…</span>`) && html('plan', bare).includes(`title="${bare}"`));
  // Counted in characters, not code units.
  const jp = 'あ'.repeat(16);
  assert.ok(html('plan', jp).includes(`<span>${jp}</span>`));
  assert.ok(html('plan', 'あ'.repeat(17)).includes(`<span>${'あ'.repeat(15)}…</span>`));
});

test('with no choices the kind has one primary button, only when its decision is in options', () => {
  const only = (kind, options) => qcAnswerButtons({ kind, options });
  assert.ok(only('plan', ['approve', 'changes'])[0].html.includes('この計画で進める'));
  assert.ok(only('diff', ['approve'])[0].html.includes('PR にする'));
  assert.ok(only('verify', ['approve'])[0].html.includes('確認した'));
  assert.ok(only('result', ['ack'])[0].html.includes('分かった（閉じる）'));
  assert.ok(only('question', ['answer'])[0].html.includes('答える'));
  assert.ok(only('issue', ['approve'])[0].html.includes('起票する') && only('relay', ['approve'])[0].html.includes('Jules に回す'));
  assert.deepEqual(only('plan', ['changes']), []);
  assert.deepEqual(only('diff', []), []);
  assert.ok(only('other', ['answer', 'approve'])[0].html.includes('data-act="approve"'));
  const c = card({ id: 'd9', kind: 'diff', title: 'D', wait: true, options: ['changes'], diff: DIFF });
  assert.ok(!c.foot.includes('PR にする') && c.foot.includes('直してほしい'));
});

test('a research result is 調査結果 with the conclusion first and bold; buttons follow the choices', () => {
  const g = {
    id: 'r2', kind: 'result', title: 'R', wait: true, options: ['ack', 'ask', 'task'], focus: 'Enough?',
    sections: [{ heading: 'Conclusion', body: 'It is the cache' }, { heading: 'Cause', body: 'stale keys' }],
  };
  const c = card(g);
  assert.ok(c.body.includes('調査結果') && !c.body.includes('調べたこと'));
  assert.ok(c.body.includes('<div class="qc-concl">') && c.body.indexOf('qc-concl') < c.body.indexOf('It is the cache'));
  assert.ok(!c.body.includes('Conclusion') && c.body.includes('Cause'));
  assert.ok(c.body.indexOf('It is the cache') < c.body.indexOf('stale keys'));
  for (const label of ['分かった（閉じる）', '続けて聞く', 'これをタスクにする']) assert.ok(c.foot.includes(label), label);
  // A result with a plan is the same as before.
  const planned = card({ ...g, sections: [...g.sections, { heading: 'Next', body: 'n', stage: 'plan' }] });
  assert.ok(planned.body.includes('調べたこと') && !planned.body.includes('qc-concl'));
  // With choices: 「もう少し調べて」, and no task button.
  const withChoices = card({ ...g, choices: [{ id: 'a', label: 'A. fix it', recommended: true }, { id: 'end', label: 'B. end' }] });
  assert.ok(withChoices.foot.includes('もう少し調べて') && !withChoices.foot.includes('続けて聞く') && !withChoices.foot.includes('これをタスクにする'));
});

test('a dispatch card reads the hub\'s reading as a table, then the question, with 着手する / 直す and the text button 「やめる」', () => {
  const g = {
    id: 'h1', kind: 'dispatch', title: 'Start', wait: true, options: ['approve', 'changes', 'reject'], focus: 'Start on this reading?',
    facts: ['やること: add a timer', 'Issue: https://github.com/acme/widget/issues/7', 'a plain fact'],
    decided: '- 完了条件: tests pass\n* 止める所：PR まで\nnot a pair\n- a very long label that is not a label: v',
    unsure: 'scope', sections: [{ heading: 'Extra', body: 'more' }],
  };
  const c = card(g);
  assert.ok(c.body.includes('あなたの依頼') && c.body.includes('hub の読み取り'));
  assert.ok(!c.body.includes('調べたこと') && !c.body.includes('>計画<'));
  assert.ok(c.body.includes('<dl class="qc-kv">'));
  assert.ok(c.body.includes('<dt>やること</dt><dd>add a timer</dd>') && c.body.includes('<dt>完了条件</dt><dd>tests pass</dd>') && c.body.includes('<dt>止める所</dt><dd>PR まで</dd>'));
  assert.ok(c.body.includes('<dt>Issue</dt><dd><a href="https://github.com/acme/widget/issues/7"'));
  assert.equal(count(c.body, '<dt>'), 4);
  assert.ok(c.body.includes('a plain fact') && c.body.includes('not a pair') && c.body.includes('a very long label'));
  assert.ok(c.body.indexOf('<dl class="qc-kv">') < c.body.indexOf('a plain fact') && c.body.includes('懸念点') && c.body.includes('Extra'));
  assert.ok(c.body.indexOf('hub の読み取り') < c.body.indexOf('qc-ask') && c.body.includes('Start on this reading?'));
  assert.ok(c.foot.includes('着手する') && c.foot.includes('data-act="changes"') && c.foot.includes('>直す<'));
  assert.ok(c.foot.includes('placeholder="直してほしいところ（任意）"'));
  assert.ok(/class="btn-m3-text reject" data-act="reject"/.test(c.foot) && c.foot.includes('>やめる<'));
  assert.ok(c.foot.indexOf('qc-spacer') < c.foot.indexOf('data-act="reject"'));
  // Values from a gate are escaped.
  assert.ok(!card({ ...g, facts: ['x: <script>1</script>'] }).body.includes('<script>'));
  // The other kinds say a send-back needs a comment.
  assert.ok(card(planGate).foot.includes('直してほしいときは必須'));
});

test('qcNeedsComment is true for the two send-backs only', () => {
  assert.equal(qcNeedsComment('changes'), true);
  assert.equal(qcNeedsComment('ask'), true);
  for (const act of ['approve', 'reject', 'ack', 'task', 'answer', 'choice', 'close', 'talk']) assert.equal(qcNeedsComment(act), false, act);
});

test('a quick row takes its icon by what it asks', () => {
  assert.equal(qcQuickIcon({ kind: 'question', choices: [{ id: 'cleanup' }, { id: 'keep' }] }), 'cleaning_services');
  assert.equal(qcQuickIcon({ kind: 'permission' }), 'front_hand');
  assert.equal(qcQuickIcon({ kind: 'question', choices: [{ id: 'y' }] }), 'bolt');
  assert.ok(quickRowHtml({ id: 'k', kind: 'question', quick: true, focus: 'Clean?', choices: [{ id: 'cleanup', label: 'Yes' }] }).includes('>cleaning_services<'));
  assert.ok(quickRowHtml({ id: 'k', kind: 'permission', quick: true, focus: 'Allow?', options: ['answer'] }).includes('>front_hand<'));
  assert.ok(quickRowHtml({ id: 'k', kind: 'question', quick: true, focus: 'Q', choices: [{ id: 'y', label: 'Yes' }] }).includes('>bolt<'));
});

test('a question with choices keeps a way to answer in words; without choices it has the one 答える', () => {
  const q = { id: 'q1', kind: 'question', title: 'Q', wait: true, options: ['answer'], focus: 'Which?' };
  const withChoices = card({ ...q, choices: [{ id: 'a', label: 'A. One', recommended: true }, { id: 'b', label: 'B. Two' }] });
  assert.ok(withChoices.foot.includes('data-act="answer"') && withChoices.foot.includes('書いて答える'));
  assert.ok(withChoices.foot.indexOf('data-choice="a"') < withChoices.foot.indexOf('書いて答える'));
  const plain = card(q);
  assert.ok(plain.foot.includes('答える') && !plain.foot.includes('書いて答える'));
});

test('a result of a task with an approved plan is a report on that plan, not a research result', () => {
  const r = { id: 'r9', kind: 'result', title: 'R', wait: true, options: ['ack', 'ask', 'task'], focus: 'ok?', sections: [{ heading: '結論', body: 'Done' }] };
  const approved = { id: 'ap9', kind: 'plan', title: 'P', steps: [{ text: 'step' }], wait: false };
  const c = card(r, { plan: approved });
  assert.ok(!c.body.includes('調査結果') && c.body.includes('調べたこと') && c.body.includes('step'));
  assert.ok(card(r).body.includes('調査結果'));
});

test('a long label is named by its id only when the id is marked off; a dashed fact is a row of the reading', () => {
  const g = { id: 'p8', kind: 'plan', title: 'P', wait: true, options: ['approve', 'changes'], focus: 'which?',
    choices: [{ id: 'a', label: 'A) keep the cache as it is today' }, { id: 'b', label: 'I want to keep the cache as it is' }] };
  const foot = card(g).foot;
  assert.ok(foot.includes('>A で進める<') && !foot.includes('I で進める') && foot.includes('I want to keep …'));
  const d = { id: 'd8', kind: 'dispatch', title: 'D', wait: true, options: ['approve', 'changes', 'reject'], facts: ['- やること: add a timer'] };
  const body = card(d).body;
  assert.ok(body.includes('<dt>やること</dt>') && !body.includes('- やること'));
});
