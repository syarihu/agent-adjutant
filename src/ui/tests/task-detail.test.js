/* The pure half of the task detail (src/ui/task-detail.js), run with `node --test`. The page's scripts share one global scope, so they
   are loaded into one context the way the browser does; what needs the page (the panel's renderers, the timeline) is stubbed or cut out
   of its file. */
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

const ctx = vm.createContext({ TextEncoder, TextDecoder, URL });
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) run(read(file), file);
const board = read('board.js');
const taskView = read('task-view.js');
const decide = read('decide.js');
run([
  cut(board, /^const DONE_WHEN = [^\n]*\n/m),
  cut(board, /^const STOP_AT = [^\n]*\n/m),
  cut(taskView, /^function issueNumberOf\(url\) \{[\s\S]*?\n\}/m),
  cut(taskView, /^function prNumberOf\(url\) \{[\s\S]*?\n\}/m),
  cut(taskView, /^const prRefNumber = [^\n]*\n/m),
  cut(decide, /^const gateRef = [^\n]*\n/m),
  // The renderers that need the page's state are stood in for by a mark each.
  'const timelineHtml = () => "<ol class=\\"timeline\\">TIMELINE</ol>";',
  'const roundsCardHtml = g => (g.reviewRounds || []).length ? "<i>ROUNDS</i>" : "";',
  'const findingsCardHtml = g => (g.findings || []).length ? "<i>FINDINGS</i>" : "";',
  'const filesTableHtml = g => g.diff ? "<i>FILES</i>" : "";',
  'const diffCardHtml = g => g.diff ? "<i>DIFF</i>" : "";',
  'const commandsCardHtml = g => (g.commands || []).length ? "<i>COMMANDS</i>" : "";',
  'const manualCardHtml = g => (g.manual || []).length ? "<i>MANUAL</i>" : "";',
].join('\n'), 'stubs');
run(read('question-card.js'), 'question-card.js');
run(read('task-detail.js'), 'task-detail.js');
const T = vm.runInContext(`({ tocDialogHtml, tocActiveKey, taskDetailSub, taskDetailNowText, taskOverviewHtml, taskChangesHtml, taskHistoryHtml,
  taskDetailKvHtml, taskDetailGroups, taskDetailPickText })`, ctx);

const count = (h, needle) => h.split(needle).length - 1;
const task = {
  id: 't1', title: 'Add a CSV export', body: 'Please add **CSV** export', status: 'dispatched',
  issueUrl: 'https://github.com/acme/widget/issues/12', pr: 'https://github.com/acme/widget/pull/15',
  issueSnapshot: { title: 'CSV', body: 'Issue body' }, doneWhen: 'pr', stopAt: 'diff',
  worktree: '/tmp/wt', branch: 'feat/csv', base: 'main', instruction: 'be careful\nsecond line', createdAt: '20260922T041233Z',
};
const plan = {
  id: 'p1', kind: 'plan', title: 'Plan', goal: 'Export rows', wait: true, decision: 'approve', answeredAt: '20260922T051233Z',
  openedAt: '20260922T041300Z', steps: [{ text: 'Write <b>it</b>', detail: 'in one file' }, { text: 'Test' }],
};

test('the frame has the sub line, the title, the now line, a table of contents and one section per group', () => {
  const f = T.tocDialogHtml({
    sub: ['widget', '', 'Issue #12'], title: 'A <t>', now: 'waiting <x>', idPrefix: 'p',
    groups: [{ key: 'a', icon: 'description', label: 'One', html: '<p>A</p>' }, { key: 'b', icon: 'info', label: 'Two', html: '<p>B</p>' }],
  });
  assert.ok(f.head.includes('<div class="qd-sub">widget · Issue #12</div>'));
  assert.ok(f.head.includes('<h2 class="qd-title">A &lt;t&gt;</h2>'));
  assert.ok(f.head.includes('td-now') && f.head.includes('waiting &lt;x&gt;') && f.head.includes('schedule'));
  assert.equal(count(f.body, 'data-td-go='), 2);
  assert.equal(count(f.body, 'aria-current="location"'), 1);
  assert.ok(f.body.indexOf('aria-current') < f.body.indexOf('data-td-go="b"'));
  assert.equal(count(f.body, 'data-td-group='), 2);
  assert.ok(f.body.includes('aria-labelledby="p-title-a"') && f.body.includes('id="p-title-b"'));
  assert.ok(f.body.indexOf('<p>A</p>') < f.body.indexOf('<p>B</p>'));
  assert.ok(f.body.includes('<nav class="td-toc" aria-label="目次">'));
});

test('the frame leaves out an empty now line and an empty sub line, and escapes keys and labels', () => {
  const f = T.tocDialogHtml({ title: 'T', groups: [{ key: 'k"x', icon: 'info', label: '<L>', html: '' }] });
  assert.ok(!f.head.includes('td-now') && !f.head.includes('qd-sub'));
  assert.ok(f.body.includes('data-td-go="k&quot;x"') && f.body.includes('&lt;L&gt;'));
  assert.ok(!f.body.includes('<L>'));
});

test('the table of contents marks the last group whose top has passed, and the last one at the end', () => {
  const tops = [0, 400, 900, 1500];
  assert.equal(T.tocActiveKey(tops, 0, 500, 2000), 0);
  assert.equal(T.tocActiveKey(tops, 360, 500, 2000), 1);
  assert.equal(T.tocActiveKey(tops, 359, 500, 2000), 0);
  assert.equal(T.tocActiveKey(tops, 900, 500, 2000), 2);
  assert.equal(T.tocActiveKey(tops, 1500, 500, 2000), 3);
  // Short last group: the end of the scroll is enough.
  assert.equal(T.tocActiveKey(tops, 1100, 500, 1600), 3);
  assert.equal(T.tocActiveKey(tops, 1098, 500, 1600), 3);
  assert.equal(T.tocActiveKey(tops, 1097, 500, 1600), 2);
  assert.equal(T.tocActiveKey([], 0, 10, 10), 0);
  // Nothing to scroll: the first.
  assert.equal(T.tocActiveKey(tops, 0, 500, 500), 0);
});

test('the sub line has the repository, the Issue, the PR and the phase; what is missing is left out', () => {
  assert.deepEqual(Array.from(T.taskDetailSub(task, { repo: 'widget', phase: 'PR・レビュー対応' })), ['widget', 'Issue #12', 'PR #15', 'PR・レビュー対応']);
  assert.deepEqual(Array.from(T.taskDetailSub({ id: 'x' }, { repo: '', phase: '計画' })), ['計画']);
  assert.deepEqual(Array.from(T.taskDetailSub({ pr: '#9' }, {})), ['PR #9']);
});

test('the now line: waiting gates first, then the worker, then the status', () => {
  const now = (t, o) => T.taskDetailNowText(t, o);
  assert.equal(now(task, { waiting: [{ kind: 'plan' }] }), '「設計レビュー」を待っています');
  assert.equal(now(task, { waiting: [{ kind: 'dispatch' }] }), '着手確認を待っています');
  assert.equal(now(task, { waiting: [{ kind: 'plan' }, { kind: 'verify' }] }), '確認待ちが 2 件あります（設計レビュー・動作確認）');
  assert.equal(now(task, { worker: { present: true, phase: 'implement' }, phaseMins: 5 }), 'worker は実装（5分前から）');
  assert.equal(now(task, { worker: { present: true, phase: 'implement' }, phaseMins: null }), 'worker は実装');
  assert.equal(now(task, { worker: { present: false, phase: 'implement' } }), '');
  assert.equal(now({ status: 'done' }, {}), '完了しました');
  assert.equal(now({ status: 'cancelled' }, {}), '取り消しました');
  assert.equal(now({ status: 'pr' }, { prNote: 'マージ待ち' }), 'マージ待ち');
  assert.equal(now({ status: 'pr' }, {}), '');
  assert.equal(now({ status: 'queued' }, {}), '着手前です');
  assert.equal(now({ status: 'backlog' }, {}), '着手前です');
});

test('概要 reads 依頼, ゴール, 計画, 質問と回答, いま, in that order', () => {
  const h = T.taskOverviewHtml(task, [plan], { isWaiting: () => false, plan, issueNumber: '12', nowText: 'worker は実装', waiting: [] });
  const at = s => { const i = h.indexOf(s); assert.ok(i >= 0, s); return i; };
  assert.ok(at('依頼') < at('ゴール') && at('ゴール') < at('計画') && at('計画') < at('質問と回答') && at('質問と回答') < at('>いま<'));
  assert.ok(h.includes('<strong>CSV</strong>') || h.includes('<b>CSV</b>'));
  assert.ok(h.includes('data-td-issue') && h.includes('Issue #12 の本文'));
  assert.ok(h.includes('<ol class="td-steps">') && h.includes('Write &lt;b&gt;it&lt;/b&gt;') && h.includes('in one file'));
  assert.ok(h.includes('承認済み（'));
  assert.ok(h.includes('worker は実装'));
});

test('概要 without a plan or a body: 依頼, 質問と回答 and いま remain', () => {
  const h = T.taskOverviewHtml({ id: 'x', title: 'T' }, [], { isWaiting: () => false, plan: null, nowText: '', waiting: [] });
  assert.ok(h.includes('依頼文はありません'));
  assert.ok(!h.includes('ゴール') && !h.includes('計画') && !h.includes('調べたこと'));
  assert.ok(!h.includes('data-td-issue'));
  assert.ok(h.includes('まだありません'));
  assert.equal(count(h, 'td-row'), 3);
  assert.ok(h.includes('—'));
});

test('a task whose request is only its Issue shows the Issue title; without a number the button says so', () => {
  const t = { id: 'x', issueSnapshot: { title: 'From <issue>', body: 'b' } };
  const h = T.taskOverviewHtml(t, [], { plan: null, issueNumber: '' });
  assert.ok(h.includes('From &lt;issue&gt;'));
  assert.ok(h.includes('>Issue の本文<'));
});

test('a plan waiting reads 承認待ち, a changed one 差し戻し済み, a plan without steps says so', () => {
  const o = p => T.taskOverviewHtml(task, [p], { isWaiting: g => g === p, plan: p });
  assert.ok(o({ ...plan, decision: undefined }).includes('承認待ち'));
  const changed = { ...plan, decision: 'changes' };
  assert.ok(T.taskOverviewHtml(task, [changed], { isWaiting: () => false, plan: changed }).includes('差し戻し済み'));
  const bare = { id: 'p2', kind: 'plan', title: 'Bare plan', decision: 'approve' };
  const h = T.taskOverviewHtml(task, [bare], { isWaiting: () => false, plan: bare });
  assert.ok(h.includes('Bare plan') && h.includes('手順は書かれていません'));
  assert.ok(!h.includes('ゴール'));
});

test('a task with no plan shows 調べたこと from the latest result gate', () => {
  const r1 = { id: 'r1', kind: 'result', title: 'Old', sections: [{ heading: 'Old h', body: 'old' }] };
  const r2 = { id: 'r2', kind: 'result', title: 'New', sections: [{ heading: 'New <h>', body: 'new **b**' }] };
  const h = T.taskOverviewHtml(task, [r1, r2], { isWaiting: () => false, plan: null });
  assert.ok(h.includes('調べたこと') && h.includes('<h5>New &lt;h&gt;</h5>') && !h.includes('Old h'));
  assert.ok(!h.includes('ゴール') && !h.includes('>計画<'));
  const body = T.taskOverviewHtml(task, [{ id: 'r3', kind: 'result', title: 'R', body: 'only a body' }], { plan: null });
  assert.ok(body.includes('only a body'));
  assert.ok(!T.taskOverviewHtml(task, [{ id: 'r4', kind: 'result', title: 'R' }], { plan: null }).includes('調べたこと'));
});

test('the questions answered are listed oldest first with the pick, the others, the comment and the card button', () => {
  const q1 = { id: 'q1', kind: 'question', title: 'Which?', focus: 'Pick **one**', wait: true, decision: 'choice', choice: 'b',
    choices: [{ id: 'a', label: 'Alpha' }, { id: 'b', label: 'Beta' }], answeredAt: '20260922T070000Z', comment: 'because <b>' };
  const q0 = { id: 'q0', kind: 'verify', title: 'Works?', wait: true, decision: 'changes', options: ['approve', 'changes', 'close'], answeredAt: '20260922T060000Z', comment: '' };
  const rec = { id: 'rc', kind: 'diff', title: 'Record', wait: false, decision: 'approve', answeredAt: '20260922T050000Z' };
  const open = { id: 'op', kind: 'plan', title: 'Open', wait: true, decision: 'approve', answeredAt: '20260922T080000Z' };
  const h = T.taskOverviewHtml(task, [rec, q1, q0, open], { isWaiting: g => g === open, plan: null, waiting: [open] });
  assert.equal(count(h, 'class="td-qa"'), 2);
  assert.ok(h.indexOf('Works?') < h.indexOf('Pick one'));
  assert.ok(h.includes('Beta') && h.includes('ほかの選択肢: Alpha'));
  assert.ok(h.includes('ほかの選択肢: 確認した') && !h.includes('解決済み'));
  assert.ok(h.includes('直してほしい'));
  assert.ok(h.includes('because &lt;b&gt;'));
  assert.ok(h.includes('data-td-card="q1"') && h.includes('data-td-card="q0"'));
  assert.ok(!h.includes('data-td-card="rc"') && !h.includes('data-td-card="op"'));
  assert.ok(h.includes('qc-kind'));
  // The waiting one is in いま, with the hint and no way to answer.
  assert.ok(h.includes('「設計レビュー」 Open') && h.includes('答えるのは、ターミナルの上の質問か TODO からです'));
  assert.equal(count(h, '答えるのは'), 1);
  assert.ok(!h.includes('data-act') && !h.includes('data-choice') && !h.includes('<textarea'));
});

test('a decision with no label of its own reads from DECISION; a handoff reads as what it sent', () => {
  assert.equal(T.taskDetailPickText({ kind: 'question', decision: 'closed' }), '解決済みとして閉じた');
  assert.equal(T.taskDetailPickText({ kind: 'question', decision: 'terminal' }), 'ターミナルで答えた');
  assert.equal(T.taskDetailPickText({ kind: 'plan', decision: 'approve' }), '承認する');
  assert.equal(T.taskDetailPickText({ kind: 'dispatch', decision: 'approve' }), '着手する');
  assert.equal(T.taskDetailPickText({ kind: 'plan', decision: 'choice', choice: 'gone', choices: [] }), 'gone');
  const g = { id: 'h', kind: 'verify', title: 'V', wait: true, decision: 'changes', handoff: { count: 2 }, comment: 'point one', answeredAt: '20260922T060000Z' };
  assert.equal(T.taskDetailPickText(g), 'AI 下読みの指示 2 件を worker に渡した');
  const h = T.taskOverviewHtml(task, [g], { isWaiting: () => false, plan: null });
  assert.ok(h.includes('渡した内容') && h.includes('point one'));
  assert.ok(!h.includes('qc-quote td-qa-comment'));
});

test('変更 has the facts, the review cards and the check cards, or says there is nothing', () => {
  const review = { id: 'd', kind: 'diff', facts: ['3 files'], diff: 'x', reviewRounds: [{}], findings: [{}] };
  const check = { id: 'v', kind: 'verify', commands: [{}], manual: ['look'], run: 'cargo run\nnext' };
  const h = T.taskChangesHtml(task, [review, check], { review, check });
  for (const m of ['3 files', 'PR #15', 'ROUNDS', 'FINDINGS', 'FILES', 'DIFF', 'COMMANDS', 'MANUAL', '動かし方', 'cargo run</div><div>next']) assert.ok(h.includes(m), m);
  assert.ok(h.indexOf('PR #15') < h.indexOf('ROUNDS') && h.indexOf('DIFF') < h.indexOf('COMMANDS'));
  assert.ok(T.taskChangesHtml({ id: 'x' }, [], {}).includes('まだ変更はありません'));
});

test('経過 is the timeline', () => {
  assert.ok(T.taskHistoryHtml(task, []).includes('TIMELINE'));
});

test('詳細 lists where the task lives and how it was set up, escaped, and leaves out what is empty', () => {
  const h = T.taskDetailKvHtml({ ...task, instruction: '<i>careful</i>' }, { agent: 'claude · opus' });
  assert.ok(h.startsWith('<dl class="td-kv">'));
  for (const m of ['/tmp/wt', 'feat/csv', '（main から）', 'PR作成まで', '計画と差分レビューを待つ', '&lt;i&gt;careful&lt;/i&gt;', 'claude · opus', 't1', '作成']) assert.ok(h.includes(m), m);
  assert.ok(h.includes('td-pre'));
  const bare = T.taskDetailKvHtml({ id: 'x' }, {});
  assert.ok(!bare.includes('worktree') && !bare.includes('ブランチ') && !bare.includes('申し送り') && !bare.includes('エージェント'));
  assert.ok(bare.includes('完了条件') && bare.includes('タスクID'));
});

test('the whole has the four groups in order, and the 申し送り only in 詳細', () => {
  const groups = T.taskDetailGroups(task, [plan], { isWaiting: () => false, plan, review: null, check: null });
  assert.deepEqual(Array.from(groups, g => [g.key, g.icon, g.label]), [
    ['over', 'description', '概要'], ['change', 'difference', '変更（差分・動作確認）'], ['log', 'timeline', '経過'], ['info', 'info', '詳細'],
  ]);
  assert.ok(!groups[0].html.includes('be careful'));
  assert.ok(groups[3].html.includes('be careful'));
});

test('what comes from a task or a gate is escaped', () => {
  const evil = '<img src=x onerror=1>';
  const g = { id: 'q', kind: 'question', title: evil, focus: '', wait: true, decision: 'choice', choice: 'a', choices: [{ id: 'a', label: evil }, { id: 'b', label: evil + 'b' }], answeredAt: '20260922T060000Z', comment: evil };
  const p = { ...plan, title: evil, goal: undefined, steps: [{ text: evil, detail: evil }] };
  const h = T.taskOverviewHtml({ ...task, body: '', issueSnapshot: { title: evil } }, [g, p], { isWaiting: () => false, plan: p, nowText: evil, waiting: [] })
    + T.taskDetailKvHtml({ ...task, branch: evil, worktree: evil, instruction: evil, doneWhen: evil, stopAt: evil }, { agent: evil })
    + T.tocDialogHtml({ title: evil, sub: [evil], now: evil, groups: [{ key: evil, icon: 'info', label: evil, html: '' }] }).body;
  assert.ok(!h.includes('<img'));
  assert.ok(h.includes('&lt;img'));
});
