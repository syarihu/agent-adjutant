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
// my-work-clean.js is loaded whole (it only defines); the two page helpers it asks of my-work.js and task-view.js are stood in for.
run('const workMerged = r => (r.task ? r.task.prState === "merged" : r.s?.branchPr?.state === "merged"); const prRefNumber = pr => /(\\d+)$/.exec(pr || "")?.[1] || "";', 'page.js');
run(read('my-work-clean.js'), 'my-work-clean.js');
run(read('todo.js'), 'todo.js');
const { todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoNotes, todoSlugsToRead, todoIsGone, quickRowHtml, TODO_READ_ORDER, todoCleanRowHtml } = vm.runInContext(
  '({ todoCleanRowHtml, todoItems, todoSelectable, todoPickSelection, todoListHtml, todoQuickRowHtml, todoReadRowHtml, todoNotes, todoSlugsToRead, todoIsGone, quickRowHtml, TODO_READ_ORDER })', ctx);

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

/* ── a merged session the board asks about ── */

const mergedTask = (id, extra = {}) => ({ id, title: `Task ${id}`, prState: 'merged', pr: `https://github.com/acme/widget/pull/${id.slice(1)}0`, ...extra });
const mergedSession = (id, extra = {}) => ({ id, kind: 'worker', worktree: `/w/${id}`, present: false, uncommitted: { files: 0, untracked: 0 }, unpushed: 0, phaseAt: T0, ...extra });
const mergedRow = (taskId, sid, extra = {}, task = mergedTask(taskId)) => ({ board: 'b1', task, session: mergedSession(sid, extra) });
const withRows = (rows, turns = [], nwo = 'acme/widget') => doc([{ nwo, rows, hubSessions: [], turns }]);

test('a merged session with nothing to lose is listed as a clean item with its board, session, key, repository, question and facts', () => {
  const it = todoItems({ doc: withRows([mergedRow('t1', 's1')]) });
  assert.equal(it.clean.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(it.clean[0])), { key: 'clean:b1/s1', board: 'b1', sessionId: 's1', nwo: 'acme/widget', title: 'Task t1',
    text: 'Task t1: PR #10 マージ済み。片付けてセッションを閉じる？', files: 0, unpushed: 0, since: T0 });
  assert.equal(it.count, 1);
  // Not a gate: nothing is selectable, and no board is read for it.
  assert.deepEqual(todoSelectable(it), []);
  assert.equal(todoSlugsToRead(it, { boards: {}, seen: {}, have: () => false, now: 0, freshMs: 1, force: true }).size, 0);
});

test('a clean item is counted with the gates, and the keys are the board and the session, so two boards never share one', () => {
  const d = withRows([mergedRow('t1', 's1'), { ...mergedRow('t2', 's1'), board: 'b2' }], [turn('t3', [gate('g1', 'plan', 1)])]);
  const it = todoItems({ doc: d });
  assert.deepEqual(keysOf(it.clean), ['clean:b1/s1', 'clean:b2/s1']);
  assert.equal(it.read.length, 1);
  assert.equal(it.count, 3);
});

test('the board asks only when no gate waits: not for a gate of the task, a gate the session waits on, 「残す」, another organisation, or work to lose', () => {
  const ids = (d, extra = {}) => keysOf(todoItems({ doc: d, ...extra }).clean);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1')])), ['clean:b1/s1']);
  // The worker's own clean-up gate on the task.
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1')], [turn('t1', [gate('g1', 'question', 1, { quick: true })])])), []);
  // The session waits on a gate of its own; and the same gate answered on this page no longer holds it back.
  const waiting = withRows([mergedRow('t1', 's1', { waiting: { slug: 'b1', id: 'g9', openedAt: stamp(T0) } })]);
  assert.deepEqual(ids(waiting), []);
  assert.deepEqual(ids(waiting, { isDone: key => key === 'b1/g9' }), ['clean:b1/s1']);
  // 「残す」: by the record, and by the page until the document says it.
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { cleanupKeptAt: 1 })])), []);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1')]), { kept: new Set(['clean:b1/s1']) }), []);
  // Out of scope.
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1')]), { inScope: nwo => nwo !== 'acme/widget' }), []);
  // Work to lose is for the terminal's warning only; so is a fact that is not known.
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { unpushed: 1 })])), []);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { uncommitted: { files: 0, untracked: 2 } })])), []);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { unpushed: undefined })])), []);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { uncommittedError: 'x' })])), []);
  // Not merged, and a hub.
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', {}, mergedTask('t1', { prState: 'open' }))])), []);
  assert.deepEqual(ids(withRows([mergedRow('t1', 's1', { kind: 'hub' })])), []);
});

test('clean items are oldest first by the session\'s phase time, and by title when that does not tell', () => {
  const d = withRows([mergedRow('t1', 's1', { phaseAt: T0 + 300 }), mergedRow('t2', 's2', { phaseAt: T0 + 60 }), mergedRow('t3', 's3', { phaseAt: undefined }),
    mergedRow('t4', 's4', { phaseAt: T0 + 60 }, mergedTask('t4', { title: 'A first' }))]);
  assert.deepEqual(keysOf(todoItems({ doc: d }).clean), ['clean:b1/s4', 'clean:b1/s2', 'clean:b1/s1', 'clean:b1/s3']);
});

test('the list draws the clean rows first, then the quick ones, then the gates to read, and the separator follows what is above it', () => {
  const html = it => todoListHtml(it, { gateOf: () => null });
  const rows = [mergedRow('t1', 's1')];
  const q = turn('t5', [gate('g5', 'question', 3, { quick: true })]);
  const r = turn('t6', [gate('g6', 'plan', 1)]);
  const all = html(todoItems({ doc: withRows(rows, [q, r]) }));
  assert.ok(all.indexOf('data-todo-key="clean:b1/s1"') < all.indexOf('data-todo-key="b1/g5"'));
  assert.ok(all.indexOf('data-todo-key="b1/g5"') < all.indexOf('読んで決める'));
  assert.ok(all.includes('class="todo-sep"') && !all.includes('todo-sep first'));
  // Clean rows and something to read: the separator is the one that follows rows, not the first.
  const withRead = html(todoItems({ doc: withRows(rows, [r]) }));
  assert.ok(withRead.includes('class="todo-sep"') && !withRead.includes('todo-sep first'));
  // Nothing above it: the first.
  assert.ok(html(todoItems({ doc: withRows([], [r]) })).includes('todo-sep first'));
  // Only clean rows: no separator, and the empty line is not shown.
  const only = html(todoItems({ doc: withRows(rows) }));
  assert.ok(!only.includes('todo-sep') && !only.includes('あなたの番はありません') && only.includes('data-todo-key="clean:b1/s1"'));
  assert.ok(html(todoItems({ doc: withRows([]) })).includes('あなたの番はありません'));
});

test('a clean row has the question, the two buttons carrying the board and the session, the repository, and is dimmed while a request is out', () => {
  const it = todoItems({ doc: withRows([mergedRow('t1', 's<1')]) });
  const row = todoCleanRowHtml(it.clean[0]);
  assert.ok(row.includes('Task t1: PR #10 マージ済み。片付けてセッションを閉じる？') && row.includes('class="todo-from">acme/widget</div>'));
  assert.ok(row.includes('data-wk-ask-act="cleanup"') && row.includes('data-wk-ask-act="keep"') && row.includes('data-wk-ask-board="b1"') && row.includes('data-wk-ask-sid="s&lt;1"'));
  assert.ok(row.includes('未コミット 0') && row.includes('未 push 0') && !row.includes(' disabled'));
  assert.equal(todoListHtml(it, { busyOf: key => key === 'clean:b1/s<1' }).split(' disabled').length, 3);
  const evil = todoItems({ doc: withRows([mergedRow('t1', 's1', {}, mergedTask('t1', { title: '<img onerror=x>' }))]) });
  assert.ok(!todoCleanRowHtml(evil.clean[0]).includes('<img') && todoCleanRowHtml(evil.clean[0]).includes('&lt;img'));
});
