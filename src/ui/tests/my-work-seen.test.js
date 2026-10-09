/* The pure half of 「いまの仕事」's read state (src/ui/my-work-seen.js), run with `node --test`. The page's
   scripts share one global scope, so they are loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
// Objects made inside the context have another Object.prototype: compare them as data.
const plain = x => JSON.parse(JSON.stringify(x));
const { workEntries, workItems, workActedAt, workSeenClass, workClassOf, workParkedText, workLiveItems,
  workParseMarks, workMarkMerge, workPruneMarks, workParkPatches, workNextNew } = ctx;

const stamp = secs => {
  const d = new Date(secs * 1000);
  const p = n => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}T${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}Z`;
};
const T0 = Date.UTC(2026, 9, 1) / 1000;
const SLUG = 'acme-widget';

const session = (id, extra = {}) => ({ id, kind: 'worker', present: true, hub: 'hub', worktree: `/w/${id}`, ...extra });
const agent = (status, updatedAt, extra = {}) => ({ status, updatedAt, ...extra });
const waiting = (id, openedAt, kind = 'plan', slug = SLUG) => ({ id, kind, slug, hub: 'hub', openedAt: stamp(openedAt), count: 1 });
const task = (id, extra = {}) => ({ id, title: `Task ${id}`, status: 'dispatched', waitsOnPerson: false, ...extra });
const docOf = repo => ({ now: T0 + 1000, repos: [{ nwo: 'acme/widget', carrier: SLUG, hubs: [], rows: [], hubSessions: [], turns: [], ...repo }] });
const row = (s, t = null, board = SLUG) => ({ board, session: s, ...(t ? { task: t } : {}) });
const one = (repo, done) => workEntries(docOf(repo), done);
const byId = (entries, id) => entries.find(e => e.id === id);

test('a gate that opens is new, leaving does not read it, reading does, and a newer gate makes it new again', () => {
  const gate = [{ kind: 'gate', gate: 'plan', since: T0 + 10 }];
  assert.equal(workSeenClass(gate, {}), 'new');
  // `left` alone leaves a gate new: opening a row and leaving it reads nothing.
  assert.equal(workSeenClass(gate, { left: T0 + 20 }), 'new');
  assert.equal(workSeenClass(gate, { read: T0 + 20 }), null);
  const two = [...gate, { kind: 'gate', gate: 'diff', since: T0 + 30 }];
  assert.equal(workSeenClass(two, { read: T0 + 20 }), 'new');
});

test('a legacy ✓ (cleared) counts as read', () => {
  const gate = [{ kind: 'gate', gate: 'plan', since: T0 + 10 }];
  assert.equal(workSeenClass(gate, { cleared: T0 + 20 }), null);
  assert.equal(workSeenClass(gate, { cleared: T0 + 5 }), 'new');
});

test('a phase moving on, or anything else that is not an item, leaves a read row where it is', () => {
  const entries = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10), phase: 'plan', agentSession: agent('done', T0 + 5) }), task('1'))] });
  const e = byId(entries, `${SLUG}/1`);
  const marks = { [e.id]: { read: T0 + 20 } };
  assert.equal(workClassOf(e, marks), null);
  const moved = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10), phase: 'implement', agentSession: agent('done', T0 + 5, { activity: 'Edit', contextPercent: 40 }) }), task('1'))] });
  assert.equal(workClassOf(byId(moved, e.id), marks), null);
});

test('a permission wait whose status time moved is new again, and one that did not is not', () => {
  const at = t => byId(one({ rows: [row(session('w1', { agentSession: agent('waiting', t, { request: 'Bash: make' }) }), task('1'))] }), `${SLUG}/1`);
  const marks = { [`${SLUG}/1`]: { read: T0 + 20 } };
  assert.equal(workClassOf(at(T0 + 10), marks), null);
  assert.equal(workClassOf(at(T0 + 30), marks), 'new');
});

test('a failed session is an item, and so is a worker whose turn ended with no gate open', () => {
  const e = byId(one({ rows: [row(session('w1', { agentSession: agent('failed', T0 + 5) }), task('1')), row(session('w2', { agentSession: agent('done', T0 + 6) }), task('2'))] }), `${SLUG}/2`);
  assert.deepEqual(plain(e.items), [{ kind: 'done', since: T0 + 6 }]);
  const f = one({ rows: [row(session('w1', { agentSession: agent('failed', T0 + 5) }), task('1'))] })[0];
  assert.deepEqual(plain(f.items), [{ kind: 'failed', since: T0 + 5 }]);
});

test('an answered gate or typing in the terminal counts as having read, with no read mark', () => {
  const gate = [{ kind: 'gate', gate: 'plan', since: T0 + 10 }];
  assert.equal(workSeenClass(gate, {}, T0 + 20), null);
  const e = byId(one({ rows: [row(session('w1', { agentSession: agent('done', T0 + 10, { lastPromptAt: T0 + 15 }) }), task('1'))] }), `${SLUG}/1`);
  assert.equal(workActedAt(e), T0 + 15);
  assert.equal(workClassOf(e, {}), null);
  const answered = byId(one({ rows: [row(session('w1', { agentSession: agent('done', T0 + 10) }), task('1', { gateAnsweredAt: stamp(T0 + 12) }))] }), `${SLUG}/1`);
  assert.equal(workActedAt(answered), T0 + 12);
  assert.equal(workClassOf(answered, {}), null);
  // The session that was not shown (a gone one) has nothing to type into.
  const gone = byId(one({ rows: [row(session('w1', { present: false, agentSession: agent('done', T0 + 10, { lastPromptAt: T0 + 15 }) }), task('1'))] }), `${SLUG}/1`);
  assert.equal(workActedAt(gone), -Infinity);
});

test('a row sent back is new until it is read again', () => {
  const gate = [{ kind: 'gate', gate: 'plan', since: T0 + 10 }];
  assert.equal(workSeenClass(gate, { read: T0 + 20, back: T0 + 25 }), 'new');
  assert.equal(workSeenClass(gate, { read: T0 + 30, back: T0 + 25 }), null);
  // Leaving after a send back does not read it.
  assert.equal(workSeenClass(gate, { read: T0 + 20, back: T0 + 25, left: T0 + 30 }), 'new');
});

test('reading drops done and failed items older than it, and keeps a gate, a permission wait and a PR', () => {
  const finished = [{ kind: 'done', since: T0 + 1 }, { kind: 'failed', since: T0 + 2 }];
  const pr = [{ kind: 'pr', turn: 'ci-failed', since: T0 + 3 }, { kind: 'pr', turn: 'merge', since: null }];
  assert.equal(workSeenClass(finished, { read: T0 + 5 }), null);
  assert.equal(workLiveItems(finished, { read: T0 + 5 }).length, 0);
  // The legacy ✓ drops them alike.
  assert.equal(workLiveItems(finished, { cleared: T0 + 5 }).length, 0);
  // Something newer after the read is not dropped by it.
  assert.equal(workSeenClass([...finished, { kind: 'failed', since: T0 + 9 }], { read: T0 + 5 }), 'new');
  // A PR still waits on the person, so it stays live (it only stops being new).
  assert.deepEqual(plain(workLiveItems(pr, { read: T0 + 5 })), plain(pr));
  assert.equal(workSeenClass(pr, { read: T0 + 5 }), null);
  const kept = [{ kind: 'gate', gate: 'plan', since: T0 + 1 }, { kind: 'permission', since: T0 + 1 }];
  assert.equal(workLiveItems([...finished, ...pr, ...kept], { read: T0 + 5 }).length, 4);
  // Acting on the row (typing into its session) drops them as reading does.
  assert.equal(workLiveItems(finished, {}, T0 + 5).length, 0);
  assert.equal(workLiveItems(finished, {}, T0 + 1).length, 1);
  // Not read yet: everything is live.
  assert.equal(workLiveItems([...finished, ...pr, ...kept], {}).length, 6);
});

test('with no item on it a row is in neither', () => {
  assert.equal(workSeenClass([], {}), null);
  const e = byId(one({ rows: [row(session('w1', { agentSession: agent('running', T0 + 1) }), task('1'))] }), `${SLUG}/1`);
  assert.equal(workClassOf(e, {}), null);
  // Idle is nothing to do, and a hub that finished a turn has nothing to clear.
  const idle = one({ rows: [row(session('w1', { agentSession: agent('idle', T0 + 1) }), task('1')), row(session('hub', { kind: 'hub', agentSession: agent('done', T0 + 2) }))] });
  assert.deepEqual(idle.map(e => e.items.length), [0, 0]);
});

test('a failed hub and a hub waiting for a person are items', () => {
  const e = one({ rows: [row(session('hub', { kind: 'hub', agentSession: agent('failed', T0 + 4), waiting: waiting('g1', T0 + 3) }))] })[0];
  assert.equal(e.id, `${SLUG}/hub:hub`);
  assert.deepEqual(plain(e.items.map(i => i.kind)), ['gate', 'failed']);
});

test('a PR that flaps is new again when its turn time is newer, and one with no time is new until opened once', () => {
  const at = prTurnAt => byId(one({ rows: [row(session('w1', { present: false }), task('1', { waitsOnPerson: true, prTurn: 'ci-failed', prTurnAt }))] }), `${SLUG}/1`);
  const marks = { [`${SLUG}/1`]: { read: T0 + 50 } };
  assert.equal(workClassOf(at(stamp(T0 + 10)), marks), null);
  assert.equal(workClassOf(at(stamp(T0 + 60)), marks), 'new');
  assert.equal(workClassOf(at(undefined), {}), 'new');
  assert.equal(workClassOf(at(undefined), marks), null);
  // A turn that is not the person's is no item.
  const checks = byId(one({ rows: [row(session('w1'), task('1', { waitsOnPerson: false, prTurn: 'checks' }))] }), `${SLUG}/1`);
  assert.equal(checks.items.length, 0);
});

test('a gate in the turns and the same one as a session waits on is one item, on the row of its task', () => {
  const g = { id: 'g1', kind: 'plan', slug: SLUG, openedAt: stamp(T0 + 10), task: '1' };
  const entries = one({
    rows: [row(session('w1', { waiting: waiting('g1', T0 + 10), agentSession: agent('done', T0 + 5) }), task('1'))],
    turns: [{ board: SLUG, task: task('1'), gates: [g] }],
  });
  assert.equal(entries.length, 1);
  assert.deepEqual(plain(entries[0].items), [{ kind: 'gate', gate: 'plan', title: '', since: T0 + 10, key: `${SLUG}/g1` }]);
  // With a gate open, the turn that ended is not a thing to clear.
  assert.ok(!entries[0].items.some(i => i.kind === 'done'));
});

test('a turn with no session is an entry of its own, merged into a row of the same task when there is one', () => {
  const g = { id: 'g2', kind: 'dispatch', slug: SLUG, openedAt: stamp(T0 + 10), task: '5' };
  const turns = [
    { board: SLUG, task: task('5'), gates: [g] },
    { board: SLUG, task: task('6', { waitsOnPerson: true, prTurn: 'merge' }), gates: [] },
    { board: SLUG, gates: [{ id: 'g9', kind: 'question', slug: SLUG, openedAt: stamp(T0 + 11) }] },
  ];
  const entries = one({ rows: [row(session('w1'), task('6', { waitsOnPerson: true, prTurn: 'merge' }))], turns });
  const ids = entries.map(e => e.id).sort();
  // The task-less gate no session waits on has a row of its own.
  assert.deepEqual(ids, [`${SLUG}/5`, `${SLUG}/6`, `${SLUG}/gate:${SLUG}/g9`]);
  assert.equal(byId(entries, `${SLUG}/5`).session, null);
  assert.deepEqual(plain(byId(entries, `${SLUG}/6`).items.map(i => i.kind)), ['pr']);
  // The session that waits on the task-less gate gets it.
  const with_ = one({ rows: [row(session('w3', { waiting: waiting('g9', T0 + 11, 'question') }))], turns });
  assert.deepEqual(plain(byId(with_, `${SLUG}/session:w3`).items.map(i => i.key)), [`${SLUG}/g9`]);
});

test('a gate answered on this page is no item', () => {
  const entries = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1'))] }, key => key === `${SLUG}/g1`);
  assert.equal(entries[0].items.length, 0);
});

test('a parent-task hub that waits is an entry on its own board', () => {
  const entries = one({ hubSessions: [{ board: 'acme-widget-wid-9', session: session('hub-wid-9', { kind: 'hub', waiting: waiting('g1', T0 + 10, 'plan', 'acme-widget-wid-9') }) }] });
  assert.equal(entries[0].id, 'acme-widget-wid-9/hub:hub-wid-9');
  assert.equal(entries[0].items.length, 1);
});

test('of two sessions of one task the entry takes the live one', () => {
  const entries = one({ rows: [row(session('old', { present: false }), task('1')), row(session('live', { agentSession: agent('failed', T0 + 1) }), task('1'))] });
  assert.equal(entries.length, 1);
  assert.equal(entries[0].session.id, 'live');
});

test('marks read from storage are only what is made of finite times', () => {
  assert.deepEqual(plain(workParseMarks('{{ not json')), {});
  assert.deepEqual(plain(workParseMarks('null')), {});
  assert.deepEqual(plain(workParseMarks('[1,2]')), {});
  assert.deepEqual(plain(workParseMarks('"x"')), {});
  const got = workParseMarks(JSON.stringify({ a: { left: 5, read: 6, back: 'x', cleared: null }, b: 7, c: { left: 1e999 }, d: [], __proto__x: { left: 1 } }));
  assert.deepEqual(plain(got), { a: { left: 5, read: 6 }, __proto__x: { left: 1 } });
  const evil = workParseMarks('{"__proto__": {"left": 1}}');
  assert.deepEqual(plain(evil), {});
  assert.equal(({}).left, undefined);
});

test('a mark moves only later, so a write from another tab is never undone by an older one', () => {
  const first = workMarkMerge({}, 'a', { left: 10 });
  const second = workMarkMerge(first, 'a', { left: 5, back: 7 });
  assert.deepEqual(plain(second), { a: { left: 10, back: 7 } });
  assert.deepEqual(plain(first), { a: { left: 10 } });
  assert.deepEqual(plain(workMarkMerge(second, 'b', { cleared: NaN })), { a: { left: 10, back: 7 }, b: {} });
});

test('marks of rows no longer listed are kept for a week and then dropped', () => {
  const now = T0 + 100 * 86400;
  const marks = { live: { left: 1 }, recent: { left: now - 86400 }, old: { left: now - 8 * 86400, back: now - 9 * 86400 }, 'old-but-newest-back': { left: 1, back: now - 86400 }, 'old-but-newest-read': { left: 1, read: now - 86400 } };
  const kept = workPruneMarks(marks, new Set(['live']), now);
  assert.deepEqual(Object.keys(plain(kept)).sort(), ['live', 'old-but-newest-back', 'old-but-newest-read', 'recent']);
});

test('a gate no session waits on is on the hub of its board, else a row of its own', () => {
  const g = { id: 'g9', kind: 'question', slug: SLUG, title: 'Which way?', openedAt: stamp(T0 + 11) };
  // The hub session waits on another gate: the stray one is the hub's too.
  const withHub = one({
    rows: [row(session('hub', { kind: 'hub', waiting: waiting('g1', T0 + 3) }))],
    turns: [{ board: SLUG, gates: [g, { id: 'g1', kind: 'plan', slug: SLUG, openedAt: stamp(T0 + 3) }] }],
  });
  assert.equal(withHub.length, 1);
  assert.deepEqual(plain(withHub[0].items.map(i => i.key).sort()), [`${SLUG}/g1`, `${SLUG}/g9`]);
  // No hub row at all: an entry keyed by the gate, which carries its title.
  const alone = one({ turns: [{ board: SLUG, gates: [g] }] });
  assert.equal(alone.length, 1);
  assert.equal(alone[0].id, `${SLUG}/gate:${SLUG}/g9`);
  assert.equal(alone[0].gateTitle, 'Which way?');
  assert.equal(workClassOf(alone[0], {}), 'new');
  // Answered on this page: nothing.
  assert.equal(one({ turns: [{ board: SLUG, gates: [g] }] }, () => true).length, 0);
});

const park = (since, extra = {}) => ({ reason: 'pdm', text: '', since: stamp(since), ...extra });
const withMarks = (marks, patches) => patches.reduce((m, [id, p]) => workMarkMerge(m, id, p), marks);

test('a parked task is never new, even with an unread gate and no marks', () => {
  const entries = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1', { parked: park(T0 + 5) }))] });
  const e = byId(entries, `${SLUG}/1`);
  assert.equal(workClassOf(e, {}), null);
  // Not even for a mark that sent it back, or a gate newer than the park.
  assert.equal(workClassOf(e, { [e.id]: { back: T0 + 500 } }), null);
  assert.deepEqual(plain(e.items.map(i => i.kind)).sort(), ['gate', 'parked']);
});

test('a parked task with nothing waiting and no session is an entry, and a finished one is not parked', () => {
  const entries = one({ turns: [{ board: SLUG, task: task('1', { parked: park(T0 + 5) }), gates: [] }] });
  const e = byId(entries, `${SLUG}/1`);
  assert.deepEqual(plain(e.items), [{ kind: 'parked', reason: 'pdm', text: '', since: T0 + 5 }]);
  assert.equal(workClassOf(e, {}), null);
  // It is still live, so the row stays listed.
  assert.equal(workLiveItems(e.items, {}).length, 1);
  const done = one({ turns: [{ board: SLUG, task: task('2', { status: 'done', parked: park(T0 + 5) }), gates: [] }] });
  assert.deepEqual(plain(byId(done, `${SLUG}/2`).items), []);
  assert.equal(workClassOf(byId(done, `${SLUG}/2`), {}), null);
  // A blank reason is no park.
  const blank = one({ turns: [{ board: SLUG, task: task('3', { parked: park(T0 + 5, { reason: '' }) }), gates: [] }] });
  assert.deepEqual(plain(byId(blank, `${SLUG}/3`).items), []);
});

test('a parked row says why, with the text when there is one, and how long ago', () => {
  assert.equal(workParkedText({ kind: 'parked', reason: 'pdm', text: '', since: 1 }), '置いている — PdM の確認待ち');
  assert.equal(workParkedText({ kind: 'parked', reason: 'other', text: '法務の返事', since: 1 }), '置いている — その他（法務の返事）');
  // With the time, the age follows the park's words.
  assert.equal(workParkedText({ kind: 'parked', reason: 'pdm', text: '', since: T0 }, T0 + 7200), '置いている — PdM の確認待ち · 2時間前から');
  assert.equal(workParkedText({ kind: 'parked', reason: 'pdm', text: '', since: T0 }, T0 + 10), '置いている — PdM の確認待ち · たった今から');
  // A reason this page does not know is shown as it was written.
  assert.equal(workParkedText({ kind: 'parked', reason: 'legal', text: '', since: 1 }), '置いている — legal');
});

test('a park is written once, and taking it off sends a row with something open back to new, once', () => {
  const parked = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1', { parked: park(T0 + 20) }))] });
  const e = byId(parked, `${SLUG}/1`);
  let marks = {};
  const first = plain(workParkPatches(parked, marks, T0 + 100));
  assert.deepEqual(first, [[e.id, { parked: T0 + 20 }]]);
  marks = withMarks(marks, first);
  assert.deepEqual(plain(workParkPatches(parked, marks, T0 + 110)), []);

  // The park is taken off while the gate is still open.
  const off = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1'))] });
  const back = plain(workParkPatches(off, marks, T0 + 200));
  assert.deepEqual(back, [[e.id, { back: T0 + 200 }]]);
  marks = withMarks(marks, back);
  assert.equal(workClassOf(byId(off, e.id), marks), 'new');
  assert.deepEqual(plain(workParkPatches(off, marks, T0 + 210)), []);

  // Read after that, it is not new; a second park is a new one.
  marks = workMarkMerge(marks, e.id, { read: T0 + 300 });
  assert.equal(workClassOf(byId(off, e.id), marks), null);
  const again = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1', { parked: park(T0 + 400) }))] });
  assert.deepEqual(plain(workParkPatches(again, marks, T0 + 410)), [[e.id, { parked: T0 + 400 }]]);
});

test('a row parked while it is open leaves new on the next document, and a task with nothing left after the park is in neither', () => {
  const open = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1'))] });
  const e = byId(open, `${SLUG}/1`);
  assert.equal(workClassOf(e, {}), 'new');
  const parked = one({ rows: [row(session('w1', { waiting: waiting('g1', T0 + 10) }), task('1', { parked: park(T0 + 20) }))] });
  assert.equal(workClassOf(byId(parked, e.id), {}), null);
  // Nothing open once the park is off: no item, so not new, whatever `back` says.
  const bare = one({ turns: [{ board: SLUG, task: task('2', { parked: park(T0 + 20) }), gates: [] }] });
  const id = byId(bare, `${SLUG}/2`).id;
  const marks = withMarks({}, workParkPatches(bare, {}, T0 + 30));
  const off = one({ turns: [{ board: SLUG, task: task('2'), gates: [] }] });
  const back = workParkPatches(off, marks, T0 + 40);
  assert.equal(workClassOf(byId(off, id), withMarks(marks, back)), null);
});

test('the parked mark is kept by parse and counted by prune', () => {
  const text = JSON.stringify({ a: { parked: T0, junk: 1 }, b: { parked: 'x' } });
  assert.deepEqual(plain(workParseMarks(text)), { a: { parked: T0 } });
  const kept = workPruneMarks({ a: { parked: T0 } }, new Set(), T0 + 86400);
  assert.deepEqual(plain(kept), { a: { parked: T0 } });
  assert.deepEqual(plain(workPruneMarks({ a: { parked: T0 } }, new Set(), T0 + 30 * 86400)), {});
});

test('after a row is dealt with, the next one of 新着 is the one that follows it in the order drawn', () => {
  const order = ['a', 'b', 'c', 'd'];
  assert.equal(workNextNew(order, 'a', ['b', 'c', 'd']), 'b');
  // One that is no longer new is skipped, wherever it is.
  assert.equal(workNextNew(order, 'a', ['c', 'd']), 'c');
  assert.equal(workNextNew(order, 'b', new Set(['a', 'd'])), 'd');
});

test('with none following, the first one left opens', () => {
  const order = ['a', 'b', 'c'];
  assert.equal(workNextNew(order, 'c', ['a', 'b']), 'a');
  assert.equal(workNextNew(order, 'c', ['b']), 'b');
  assert.equal(workNextNew(order, 'b', ['a']), 'a');
});

test('with none left, nothing opens', () => {
  assert.equal(workNextNew(['a', 'b'], 'a', []), null);
  assert.equal(workNextNew(['a'], 'a', ['a']), null);
  assert.equal(workNextNew([], 'a', ['a']), null);
});

test('the row just dealt with is left out, new still or not, and one that is not in the list starts from the first', () => {
  const order = ['a', 'b', 'c'];
  // Still new (it has a second gate): not chosen again.
  assert.equal(workNextNew(order, 'b', ['a', 'b', 'c']), 'c');
  assert.equal(workNextNew(order, 'c', ['c', 'a']), 'a');
  // The row is gone from the list altogether.
  assert.equal(workNextNew(order, 'z', ['b', 'c']), 'b');
  assert.equal(workNextNew(order, 'z', []), null);
});

test('a gate whose task the board does not list gets no row keyed by that task, but its own', () => {
  const g = { id: 'g9', kind: 'plan', slug: SLUG, openedAt: stamp(T0 + 10), task: 'gone-1' };
  const entries = one({ turns: [{ board: SLUG, gates: [g] }] });
  assert.equal(entries.some(e => e.id === `${SLUG}/gone-1`), false);
  const e = entries.find(x => x.ref === 'gate:' + SLUG + '/g9');
  assert.ok(e, 'the gate is a row of its own');
  assert.equal(e.items[0].key, `${SLUG}/g9`);
  // A turn that has its task is still keyed by it.
  const listed = one({ turns: [{ board: SLUG, task: task('1'), gates: [{ ...g, task: '1' }] }] });
  assert.ok(listed.some(x => x.id === `${SLUG}/1`));
});
