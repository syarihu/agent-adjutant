/* The pure half of the TODO (src/ui/todo.js), run with `node --test`. The page's scripts share one global scope, so they are loaded into
   one context the way the browser does; the buttons and the choices are cut out of decide.js, which needs the page. */
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
run(read('todo.js'), 'todo.js');
const { todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoPrRowHtml, TODO_READ_ORDER } = vm.runInContext(
  '({ todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoPrRowHtml, TODO_READ_ORDER })', ctx);

const stamp = secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
const T0 = Date.UTC(2026, 9, 1) / 1000;
const gate = (id, kind, mins, extra = {}) => ({ id, kind, title: `${kind} ${id}`, openedAt: stamp(T0 + mins * 60), slug: 'b1', quick: false, ...extra });
const turn = (taskId, gates, board = 'b1') => ({ board, task: { id: taskId, title: `Task ${taskId}` }, gates });
const doc = (repos = [{ nwo: 'acme/widget', turns: [] }]) => ({ now: T0, repos });
const repo = (nwo, turns) => ({ nwo, rows: [], hubSessions: [], turns });
const rec = (n, extra = {}) => ({ id: `acme/widget#${n}`, repo: 'acme/widget', number: n, title: `PR ${n}`, state: 'requested', author: 'octo', requestedAt: stamp(T0 + n * 60), url: `https://github.com/acme/widget/pull/${n}`, ...extra });
const keysOf = list => list.map(x => x.key);

test('quick comes first, then the gates to read by kind and age, then the pull requests', () => {
  const d = doc([repo('acme/widget', [
    turn('t1', [gate('g1', 'diff', 1)]),
    turn('t2', [gate('g2', 'plan', 5)]),
    turn('t3', [gate('g3', 'plan', 2)]),
    turn('t4', [gate('g4', 'dispatch', 9)]),
    turn('t5', [gate('g5', 'question', 3, { quick: true })]),
    turn('t6', [gate('g6', 'question', 1, { quick: true })]),
    turn('t7', [gate('g7', 'mystery', 0)]),
  ])]);
  const it = todoItems({ doc: d, records: [rec(2), rec(1)] });
  assert.deepEqual(keysOf(it.quick), ['b1/g6', 'b1/g5']);
  assert.deepEqual(keysOf(it.read), ['b1/g4', 'b1/g3', 'b1/g2', 'b1/g1', 'b1/g7']);
  assert.deepEqual(it.prs.map(r => r.number), [1, 2]);
  assert.deepEqual(todoSelectable(it), ['b1/g4', 'b1/g3', 'b1/g2', 'b1/g1', 'b1/g7', 'pr:acme/widget#1', 'pr:acme/widget#2']);
  assert.equal(it.count, 9);
  assert.equal(TODO_READ_ORDER[0], 'dispatch');
});

test('an answered gate and a gate of a parked task are left out', () => {
  const parked = { id: 't2', title: 'Parked', parked: { reason: 'pdm' } };
  const d = doc([repo('acme/widget', [
    turn('t1', [gate('g1', 'plan', 1), gate('g2', 'plan', 2)]),
    { board: 'b1', task: parked, gates: [gate('g3', 'plan', 3)] },
  ])]);
  const it = todoItems({ doc: d, records: [], isDone: k => k === 'b1/g1' });
  assert.deepEqual(keysOf(it.read), ['b1/g2']);
  assert.equal(it.count, 1);
});

test('only the pull requests that wait on the person count, whatever their AI 下読み', () => {
  const records = [rec(1), rec(2, { state: 'ai-reading' }), rec(3, { state: 'ai-ready' }), rec(4, { state: 'pushed' }),
    rec(5, { state: 'waiting-on-author' }), rec(6, { state: 'done' })];
  const it = todoItems({ doc: doc(), records });
  assert.deepEqual(it.prs.map(r => r.number), [1, 2, 3, 4]);
  assert.equal(it.count, 4);
});

test('the scope narrows both the gates and the pull requests', () => {
  const d = doc([
    repo('acme/widget', [turn('t1', [gate('g1', 'plan', 1)])]),
    repo('other/app', [turn('t2', [gate('g2', 'plan', 2, { slug: 'b2' })], 'b2')]),
  ]);
  const records = [rec(1), rec(2, { id: 'other/app#2', repo: 'other/app' })];
  const it = todoItems({ doc: d, records, inScope: nwo => nwo.startsWith('other/') });
  assert.deepEqual(keysOf(it.read), ['b2/g2']);
  assert.deepEqual(it.prs.map(r => r.id), ['other/app#2']);
  assert.equal(it.count, 2);
});

test('a quick row has buttons once its gate is read, and says it is loading before', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]), records: [] });
  const w = it.quick[0];
  const loading = todoQuickRowHtml(w, null, {}, T0 + 600);
  assert.ok(loading.includes('読み込み中…'));
  assert.ok(!loading.includes('<button'));
  const g = { id: 'g1', kind: 'question', title: 'Which?', focus: 'Pick one', _slug: 'b1', choices: [{ id: 'a', label: 'Yes', recommended: true }, { id: 'b', label: 'No' }] };
  const html = todoQuickRowHtml(w, g, {}, T0 + 600);
  assert.ok(html.includes('data-choice="a"') && html.includes('data-choice="b"'));
  assert.ok(html.includes('data-qc-gate="b1/g1"'));
  assert.ok(html.includes('acme/widget'));
  assert.ok(html.includes('9分前から待ち'));
  // The task's title is a line of its own, which the shared quick row does not carry.
  assert.ok(html.includes('<div class="todo-task">Task t1</div>'));
});

test('a board that could not be read says so in the pending quick row, and the list passes the reason on', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]), records: [] });
  const row = todoQuickRowHtml(it.quick[0], null, {}, T0 + 600, 'HTTP 500');
  assert.ok(row.includes('このボードを読めませんでした: HTTP 500'));
  assert.ok(!row.includes('読み込み中…'));
  const list = todoListHtml(it, { errorOf: () => 'HTTP 500', now: T0 });
  assert.ok(list.includes('このボードを読めませんでした: HTTP 500'));
  assert.ok(todoListHtml(it, { now: T0 }).includes('読み込み中…'));
});

test('a row to read falls back to the kind\'s fixed question, and says what the worker asked when it is known', () => {
  const w = { key: 'b1/g1', kind: 'plan', nwo: 'acme/widget', title: 'Add a timer', since: T0 };
  const before = todoReadRowHtml(w, null, null, T0 + 7200);
  assert.ok(before.includes('data-todo-sel="b1/g1"'));
  assert.ok(before.includes('この計画で進めていい？'));
  assert.ok(before.includes('acme/widget · 2時間前から待ち'));
  assert.ok(!before.includes('aria-current'));
  const asked = todoReadRowHtml(w, { focus: 'Is **quokka** time fine?' }, 'b1/g1', T0);
  assert.ok(asked.includes('Is quokka time fine?'));
  assert.ok(asked.includes('aria-current="true"'));
});

test('a pull request row names the request, the AI 下読み stage, the repository, the author and the time', () => {
  const html = todoPrRowHtml(rec(7), null, T0 + 7 * 60 + 300, [rec(7)]);
  assert.ok(html.includes('data-todo-sel="pr:acme/widget#7"'));
  assert.ok(html.includes('#7 PR 7'));
  assert.ok(html.includes('レビュー依頼'));
  assert.ok(html.includes('未下読み'));
  assert.ok(html.includes('acme/widget · @octo · 依頼 5分前'));
});

test('the list has a separator only when there is something to read, and says so when nothing waits', () => {
  const quickOnly = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]), records: [] });
  assert.ok(!todoListHtml(quickOnly).includes('読んで決める'));
  const both = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })]), turn('t2', [gate('g2', 'plan', 2)])])]), records: [rec(1)] });
  const html = todoListHtml(both, { now: T0 });
  assert.ok(html.indexOf('todo-quick') < html.indexOf('読んで決める'));
  assert.ok(html.indexOf('読んで決める') < html.indexOf('b1/g2'));
  assert.ok(html.indexOf('b1/g2') < html.indexOf('pr:acme/widget#1'));
  const none = todoItems({ doc: doc(), records: [] });
  assert.equal(none.count, 0);
  assert.ok(todoListHtml(none).includes('あなたの番はありません'));
});

test('the selection stays, moves to the one that took its place, or starts on the first', () => {
  assert.equal(todoPickSelection('b', ['a', 'b', 'c'], ['a', 'b', 'c']), 'b');
  assert.equal(todoPickSelection('b', ['a', 'b', 'c'], ['a', 'c']), 'c');
  assert.equal(todoPickSelection('c', ['a', 'b', 'c'], ['a', 'b']), 'b');
  assert.equal(todoPickSelection(null, [], ['a', 'b']), 'a');
  assert.equal(todoPickSelection('x', ['a'], ['a', 'b']), 'a');
  assert.equal(todoPickSelection('a', ['a'], []), null);
});
