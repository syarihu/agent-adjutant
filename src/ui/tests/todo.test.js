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
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(read('question-card.js'), 'question-card.js');
run(read('todo.js'), 'todo.js');
const { todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoNotes, todoSlugsToRead, todoIsGone, quickRowHtml, TODO_READ_ORDER } = vm.runInContext(
  '({ todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoNotes, todoSlugsToRead, todoIsGone, quickRowHtml, TODO_READ_ORDER })', ctx);

const stamp = secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
const T0 = Date.UTC(2026, 9, 1) / 1000;
const gate = (id, kind, mins, extra = {}) => ({ id, kind, title: `${kind} ${id}`, openedAt: stamp(T0 + mins * 60), slug: 'b1', quick: false, ...extra });
const turn = (taskId, gates, board = 'b1') => ({ board, task: { id: taskId, title: `Task ${taskId}` }, gates });
const doc = (repos = [{ nwo: 'acme/widget', turns: [] }]) => ({ now: T0, repos });
const repo = (nwo, turns) => ({ nwo, rows: [], hubSessions: [], turns });
const rec = (n, extra = {}) => ({ id: `acme/widget#${n}`, repo: 'acme/widget', number: n, title: `PR ${n}`, state: 'requested', author: 'octo', requestedAt: stamp(T0 + n * 60), url: `https://github.com/acme/widget/pull/${n}`, ...extra });
const keysOf = list => list.map(x => x.key);

test('quick comes first, then the gates to read by kind and age; review requests are not in it', () => {
  const d = doc([repo('acme/widget', [
    turn('t1', [gate('g1', 'diff', 1)]),
    turn('t2', [gate('g2', 'plan', 5)]),
    turn('t3', [gate('g3', 'plan', 2)]),
    turn('t4', [gate('g4', 'dispatch', 9)]),
    turn('t5', [gate('g5', 'question', 3, { quick: true })]),
    turn('t6', [gate('g6', 'question', 1, { quick: true })]),
    turn('t7', [gate('g7', 'mystery', 0)]),
  ])]);
  const it = todoItems({ doc: d });
  assert.deepEqual(keysOf(it.quick), ['b1/g6', 'b1/g5']);
  assert.deepEqual(keysOf(it.read), ['b1/g4', 'b1/g3', 'b1/g2', 'b1/g1', 'b1/g7']);
  assert.equal(it.prs, undefined);
  assert.deepEqual(todoSelectable(it), ['b1/g4', 'b1/g3', 'b1/g2', 'b1/g1', 'b1/g7']);
  assert.equal(it.count, 7);
  assert.equal(TODO_READ_ORDER[0], 'dispatch');
});

test('an answered gate and a gate of a parked task are left out', () => {
  const parked = { id: 't2', title: 'Parked', parked: { reason: 'pdm' } };
  const d = doc([repo('acme/widget', [
    turn('t1', [gate('g1', 'plan', 1), gate('g2', 'plan', 2)]),
    { board: 'b1', task: parked, gates: [gate('g3', 'plan', 3)] },
  ])]);
  const it = todoItems({ doc: d, isDone: k => k === 'b1/g1' });
  assert.deepEqual(keysOf(it.read), ['b1/g2']);
  assert.equal(it.count, 1);
});

test('review requests, in whatever state, neither count nor list nor show in an otherwise empty TODO', () => {
  const records = [rec(1), rec(2, { state: 'ai-reading' }), rec(3, { state: 'ai-ready' }), rec(4, { state: 'pushed' })];
  const it = todoItems({ doc: doc(), records });
  assert.equal(it.count, 0);
  assert.deepEqual(todoSelectable(it), []);
  const html = todoListHtml(it, { records });
  assert.ok(html.includes('あなたの番はありません'));
  assert.ok(!html.includes('pr:') && !html.includes('レビュー依頼') && !html.includes('todo-pr-dot'));
});

test('the scope narrows the gates', () => {
  const d = doc([
    repo('acme/widget', [turn('t1', [gate('g1', 'plan', 1)])]),
    repo('other/app', [turn('t2', [gate('g2', 'plan', 2, { slug: 'b2' })], 'b2')]),
  ]);
  const it = todoItems({ doc: d, inScope: nwo => nwo.startsWith('other/') });
  assert.deepEqual(keysOf(it.read), ['b2/g2']);
  assert.equal(it.count, 1);
});

test('a quick row has buttons once its gate is read, and says it is loading before', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]) });
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
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]) });
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

test('the list has a separator only when there is something to read, and says so when nothing waits', () => {
  const quickOnly = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]) });
  assert.ok(!todoListHtml(quickOnly).includes('読んで決める'));
  const both = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })]), turn('t2', [gate('g2', 'plan', 2)])])]) });
  const html = todoListHtml(both, { now: T0 });
  assert.ok(html.indexOf('todo-quick') < html.indexOf('読んで決める'));
  assert.ok(html.indexOf('読んで決める') < html.indexOf('b1/g2'));
  const none = todoItems({ doc: doc() });
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

test('an error with no data is the error, and an error with old data is the data with a line', () => {
  const ok = { workDoc: {} };
  assert.deepEqual(todoNotes(ok), { blocked: null, notes: [], emptyOk: true });
  assert.equal(todoNotes({ workDoc: null, workError: 'down' }).blocked, '読み込めませんでした: down');
  assert.equal(todoNotes({ workDoc: null }).blocked, '読み込み中…');
  assert.deepEqual(todoNotes({ ...ok, workError: 'down' }).notes, ['最新を読めませんでした: down']);
});

test('nothing waiting is claimed only when the work document was read whole, and a cached quick row says its board is stale', () => {
  const none = todoItems({ doc: doc() });
  assert.equal(todoListHtml(none, { emptyOk: false }), '');
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]) });
  const g = { id: 'g1', kind: 'question', title: 'Which?', _slug: 'b1', choices: [{ id: 'a', label: 'Yes' }] };
  const html = todoListHtml(it, { gateOf: () => g, errorOf: () => 'HTTP 500', now: T0 });
  assert.ok(html.includes('data-choice="a"'));
  assert.ok(html.includes('このボードの最新を読めませんでした: HTTP 500'));
});

test('a gate its board no longer lists says it is not waiting, with no button, and a read row says so too', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true }), gate('g2', 'plan', 2)])])]) });
  const html = todoListHtml(it, { goneOf: () => true, now: T0 });
  assert.equal(html.split('もう待っていません').length - 1, 2);
  assert.ok(!html.includes('data-choice') && !html.includes('読み込み中…'));
});

test('an empty list is not claimed while a repository is unread', () => {
  const ok = { workDoc: { repos: [] } };
  assert.equal(todoNotes(ok).emptyOk, true);
  const bad = todoNotes({ ...ok, workDoc: { repos: [{ nwo: 'acme/widget', error: 'no board' }] } });
  assert.equal(bad.emptyOk, false);
  assert.ok(bad.notes[0].includes('acme/widget') && bad.notes[0].includes('no board'));
});

test('a quick row shows the first line of the question only, and the shared row keeps the whole one', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'question', 1, { quick: true })])])]) });
  const g = { id: 'g1', kind: 'question', title: 'T', focus: 'First line\nSecond line', _slug: 'b1', choices: [{ id: 'a', label: 'Yes' }] };
  const html = todoQuickRowHtml(it.quick[0], g, {}, T0);
  assert.ok(html.includes('First line') && !html.includes('Second line'));
  assert.ok(quickRowHtml(g, {}).includes('Second line'));
});

test('a board is read again when it is missing, old, forced, or read before a gate it lacks was first seen', () => {
  const it = todoItems({ doc: doc([repo('acme/widget', [turn('t1', [gate('g1', 'plan', 1)])])]) });
  const base = { boards: { b1: { data: {}, at: 1000 } }, seen: { 'b1/g1': 500 }, have: () => false, now: 2000, freshMs: 10000 };
  assert.equal(todoSlugsToRead(it, base).size, 0);
  assert.equal(todoSlugsToRead(it, { ...base, seen: { 'b1/g1': 1500 } }).size, 1);
  assert.equal(todoSlugsToRead(it, { ...base, have: () => true, seen: { 'b1/g1': 1500 } }).size, 0);
  assert.equal(todoSlugsToRead(it, { ...base, force: true }).size, 1);
  assert.equal(todoSlugsToRead(it, { ...base, now: 20000 }).size, 1);
  assert.equal(todoSlugsToRead(it, { ...base, boards: {} }).size, 1);
});

test('a gate is gone only by a read newer than when it was first seen listed', () => {
  assert.equal(todoIsGone({ data: {}, at: 1000 }, 1500, false), false);
  assert.equal(todoIsGone({ data: {}, at: 2000 }, 1500, false), true);
  assert.equal(todoIsGone({ data: {}, at: 2000 }, 1500, true), false);
  assert.equal(todoIsGone({ data: null, at: 2000 }, 1500, false), false);
});
