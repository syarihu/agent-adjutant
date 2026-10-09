/* The pure half of 離れていた間に (src/ui/my-work-away.js), run with `node --test`. Loaded into one context the way the
   browser shares its global scope, as my-work-seen.test.js does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-away.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const { workEntries, workActedAt, workAwaySince, workAwayEvents, workAwayCount, workAwayText, workLastAction, workNowText,
  workAwayModel, workParentAway } = ctx;
// `const`s of a script are in its context's scope, not on its global object.
const { secsStamp, awaySecs } = vm.runInContext('({ secsStamp, awaySecs })', ctx);

const stamp = secs => secsStamp(secs);
const T0 = Date.UTC(2026, 9, 1) / 1000;
const SLUG = 'acme-widget';
const NOW = T0 + 10000;

const session = (id, extra = {}) => ({ id, kind: 'worker', present: true, hub: 'hub', worktree: `/w/${id}`, ...extra });
const agent = (status, updatedAt, extra = {}) => ({ status, updatedAt, ...extra });
const waiting = (id, openedAt, kind = 'plan', title = 'T') => ({ id, kind, slug: SLUG, hub: 'hub', openedAt: stamp(openedAt), title, count: 1 });
const task = (id, extra = {}) => ({ id, title: `Task ${id}`, status: 'dispatched', waitsOnPerson: false, ...extra });
const docOf = repo => ({ now: NOW, repos: [{ nwo: 'acme/widget', carrier: SLUG, hubs: [], rows: [], hubSessions: [], turns: [], ...repo }] });
const row = (s, t = null) => ({ board: SLUG, session: s, ...(t ? { task: t } : {}) });
const entryOf = (s, t) => workEntries(docOf({ rows: [row(s, t)] }))[0];
const kinds = evs => evs.map(e => e.kind);

test('only what happened after the person left is an event, and acting counts as reading like leaving does', () => {
  const e = entryOf(session('w1', { phases: [['plan', T0 + 5], ['implement', T0 + 50]], agentSession: agent('failed', T0 + 60) }), task('1'));
  assert.deepEqual(kinds(workAwayEvents(e, T0 + 10)), ['phase', 'failed']);
  assert.deepEqual(kinds(workAwayEvents(e, T0 + 55)), ['failed']);
  assert.deepEqual(workAwayEvents(e, T0 + 60), []);
  // Acting later than leaving moves the line forward.
  assert.equal(workAwaySince({ left: T0 + 10 }, T0 + 55), T0 + 55);
  assert.equal(workAwaySince({ left: T0 + 70 }, T0 + 55), T0 + 70);
});

test('a task never opened and never acted on has no block and no count', () => {
  const e = entryOf(session('w1', { phases: [['plan', T0 + 5]], agentSession: agent('failed', T0 + 60) }), task('1'));
  assert.equal(workAwaySince({}, workActedAt(e)), -Infinity);
  assert.deepEqual(workAwayEvents(e, -Infinity), []);
  assert.equal(workAwayCount(e, {}, false), 0);
  assert.equal(workAwayModel(e, {}, [], NOW), null);
});

test('acted by typing into the session beats an older leave', () => {
  const e = entryOf(session('w1', { phases: [['plan', T0 + 5], ['implement', T0 + 50]], agentSession: agent('running', T0 + 1, { lastPromptAt: T0 + 40 }) }), task('1'));
  assert.equal(workAwayCount(e, { left: T0 + 10 }, false), 1);
  assert.equal(workAwayModel(e, { left: T0 + 10 }, [], NOW).last.text, 'ターミナルで指示した');
});

test('gates opened are events, a question in its own words, and a gate with no title says no title', () => {
  const e = entryOf(session('w1', { waiting: waiting('g1', T0 + 30, 'question', 'どちらにする'), agentSession: agent('done', T0 + 31) }), task('1'));
  const evs = workAwayEvents(e, T0);
  assert.deepEqual(plain(kinds(evs)), ['gate']);
  assert.equal(workAwayText(evs[0]), '質問が来た『どちらにする』');
  assert.equal(workAwayText({ kind: 'gate', gate: 'plan', title: '' }), '設計レビューが開いた');
  assert.equal(workAwayText({ kind: 'gate', gate: 'plan', title: '案' }), '設計レビューが開いた『案』');
});

test('a turn that ended is skipped while a gate is open, and says what the agent said when it is not', () => {
  const done = agent('done', T0 + 40, { lastMessage: '\n  実装を終えた。\n詳細は…', lastMessageAt: T0 + 40 });
  const withGate = entryOf(session('w1', { waiting: waiting('g1', T0 + 30), agentSession: done }), task('1'));
  assert.ok(!kinds(workAwayEvents(withGate, T0)).includes('done'));
  const plainDone = entryOf(session('w1', { agentSession: done }), task('1'));
  const [ev] = workAwayEvents(plainDone, T0);
  assert.equal(workAwayText(ev), 'worker が手を止めた — 『実装を終えた。』');
  // A message from an earlier turn is not what it said now.
  const stale = entryOf(session('w1', { agentSession: agent('done', T0 + 400, { lastMessage: '古い', lastMessageAt: T0 + 40 }) }), task('1'));
  assert.equal(workAwayText(workAwayEvents(stale, T0)[0]), 'worker が手を止めた');
});

test('a permission wait is stamped by when the status changed, not by later activity', () => {
  const e = entryOf(session('w1', { agentSession: agent('waiting', T0 + 30, { request: 'Bash: make', lastEventAt: T0 + 900 }) }), task('1'));
  assert.deepEqual(workAwayEvents(e, T0 + 20).map(workAwayText), ['許可を求めた — Bash: make']);
  assert.deepEqual(workAwayEvents(e, T0 + 31), []);
});

test('a PR change is one event by prTurnAt, none without it, and a finished task reads its state', () => {
  const at = (t, since) => workAwayEvents(entryOf(session('w1'), t), since).map(workAwayText);
  assert.deepEqual(at(task('1', { prTurn: 'changes', prTurnAt: stamp(T0 + 30) }), T0), ['PR に修正の依頼が来た']);
  assert.deepEqual(at(task('1', { prTurn: 'changes', prTurnAt: stamp(T0 + 30) }), T0 + 30), []);
  assert.deepEqual(at(task('1', { prTurn: 'changes' }), T0), []);
  assert.deepEqual(at(task('1', { prState: 'merged', prTurnAt: stamp(T0 + 30) }), T0), ['PR がマージされた']);
  assert.deepEqual(at(task('1', { prState: 'closed', prTurnAt: stamp(T0 + 30) }), T0), ['PR がマージされずに閉じられた']);
  assert.deepEqual(at(task('1', { prState: 'open', prTurnAt: stamp(T0 + 30) }), T0), []);
  for (const turn of ['draft', 'unrequested', 'other-reviewer', 'checks', 'changes', 'merge', 'ci-failed', 'merged', 'closed']) {
    assert.ok(workAwayText({ kind: 'pr', turn }).startsWith('PR '), turn);
  }
});

test('the row open counts nothing, nor does a hub or a row that is only a gate', () => {
  const e = entryOf(session('w1', { phases: [['plan', T0 + 50]] }), task('1'));
  assert.equal(workAwayCount(e, { left: T0 }, false), 1);
  assert.equal(workAwayCount(e, { left: T0 }, true), 0);
  const hub = workEntries(docOf({ hubSessions: [{ board: SLUG, session: session('hub', { kind: 'hub', phases: [['plan', T0 + 50]] }) }] }))[0];
  assert.equal(workAwayCount(hub, { left: T0 }, false), 0);
  const gate = workEntries(docOf({ turns: [{ board: SLUG, gates: [{ id: 'g9', kind: 'plan', slug: SLUG, openedAt: stamp(T0 + 50), title: 'x' }] }] }))[0];
  assert.equal(workAwayCount(gate, { left: T0 }, false), 0);
});

test('times in both shapes, epoch seconds and stamps, sort into one list', () => {
  assert.equal(awaySecs(T0), T0);
  assert.equal(awaySecs(stamp(T0)), T0);
  assert.equal(awaySecs('garbage'), null);
  assert.equal(awaySecs(null), null);
  assert.equal(stamp(T0), '20261001T000000Z');
  const e = entryOf(session('w1', {
    phases: [['implement', T0 + 20]],
    waiting: waiting('g1', T0 + 10),
    agentSession: agent('failed', T0 + 30),
  }), task('1', { prTurn: 'merge', prTurnAt: stamp(T0 + 25) }));
  const evs = workAwayEvents(e, T0);
  assert.deepEqual(plain(evs.map(x => [x.kind, x.at])), [['gate', T0 + 10], ['phase', T0 + 20], ['pr', T0 + 25], ['failed', T0 + 30]]);
});

test('the words of a phase move and of a failure', () => {
  assert.equal(workAwayText({ kind: 'phase', phase: 'verify' }), '工程が動作確認に入った');
  assert.equal(workAwayText({ kind: 'failed' }), 'worker がエラーで止まった');
});

test('the last action is the newest of an answered gate, a send-back, typing and a park, even older than the leave', () => {
  const gates = [
    { id: 'a', kind: 'plan', title: '案', decision: 'approve', answeredAt: stamp(T0 + 10) },
    { id: 'b', kind: 'diff', title: '記録', wait: false, answers: [{ answeredAt: stamp(T0 + 20), comment: 'x' }] },
    { id: 'c', kind: 'question', title: '質問', decision: 'answer', answeredAt: stamp(T0 + 5) },
  ];
  const e = entryOf(session('w1'), task('1'));
  assert.equal(workLastAction(e, gates).text, 'コードレビューの記録『記録』を差し戻した');
  assert.equal(workLastAction(e, gates.slice(0, 1)).text, '設計レビュー『案』を承認した');
  assert.equal(workLastAction(e, gates.slice(2)).text, '質問『質問』に答えた');
  const typed = entryOf(session('w1', { agentSession: agent('running', T0, { lastPromptAt: T0 + 99 }) }), task('1'));
  assert.equal(workLastAction(typed, gates).text, 'ターミナルで指示した');
  const parked = entryOf(session('w1'), task('1', { parked: { reason: 'pdm', text: '資料', since: stamp(T0 + 200) } }));
  assert.equal(workLastAction(parked, gates).text, '置いた — PdM の確認待ち（資料）');
  assert.equal(workLastAction(entryOf(session('w1'), task('1')), []), null);
});

test('an event of the current visit is left out when an upper bound is given', () => {
  const e = entryOf(session('w1', { phases: [['plan', T0 + 50], ['implement', T0 + 90]] }), task('1'));
  assert.equal(workAwayEvents(e, T0).length, 2);
  assert.deepEqual(plain(workAwayEvents(e, T0, T0 + 60).map(x => x.at)), [T0 + 50]);
  assert.equal(workAwayModel(e, { left: T0 }, [], NOW, T0 + 60).events.length, 1);
});

test('the leave is said only when it is the boundary, not when a later act is', () => {
  const e = entryOf(session('w1', { agentSession: agent('running', T0, { lastPromptAt: T0 + 100 }) }), task('1'));
  assert.equal(workAwayModel(e, { left: T0 + 60 }, [], NOW).left, null);
  assert.equal(workAwayModel(e, { left: T0 + 100 }, [], NOW).left, T0 + 100);
  const quiet = entryOf(session('w1'), task('1'));
  assert.equal(workAwayModel(quiet, { left: T0 + 60 }, [], NOW).left, T0 + 60);
});

test('the model says when the person left only when they did, and cuts the list at twenty', () => {
  const phases = Array.from({ length: 25 }, (_, i) => ['implement', T0 + 100 + i]);
  const e = entryOf(session('w1', { phases, agentSession: agent('running', T0, { lastPromptAt: T0 + 50 }) }), task('1'));
  const acted = workAwayModel(e, {}, [], NOW);
  assert.equal(acted.left, null);
  assert.equal(acted.events.length, 20);
  assert.equal(acted.more, 5);
  assert.equal(workAwayModel(e, { left: T0 + 60 }, [], NOW).left, T0 + 60);
});

test('now says the first thing the task waits on', () => {
  const now = (s, t = task('1')) => workNowText(entryOf(s, t), NOW);
  const park = { reason: 'review', since: stamp(NOW - 3 * 3600) };
  assert.equal(now(session('w1', { waiting: waiting('g1', T0), agentSession: agent('waiting', T0) }), task('1', { parked: park })), '置いている — エンジニアのレビュー待ち（3時間前から）');
  assert.equal(now(session('w1', { waiting: waiting('g1', T0), agentSession: agent('waiting', T0) })), '設計レビューがあなたの判定待ち');
  assert.equal(now(session('w1', { waiting: waiting('g1', T0, 'question') })), '質問があなたの答え待ち');
  assert.equal(now(session('w1', { agentSession: agent('waiting', T0, { request: 'Bash: ls' }) })), 'worker が許可を待っている — Bash: ls');
  assert.equal(now(session('w1', { agentSession: agent('failed', T0) })), 'worker がエラーで止まっている');
  assert.equal(now(session('w1', { agentSession: agent('done', T0) }), task('1', { waitsOnPerson: true, prTurn: 'merge' })), 'マージできます');
  assert.equal(now(session('w1', { agentSession: agent('done', T0) })), 'worker は次の指示待ち');
  assert.equal(now(session('w1')), 'worker の状態は不明');
  assert.equal(now(session('w1', { agentSession: { error: 'boom' } })), 'worker の状態は不明');
  assert.equal(now(session('w1', { phase: 'implement', agentSession: agent('brand-new', T0) })), 'worker の状態は不明');
  assert.equal(now(session('w1', { phase: 'implement', phaseAt: NOW - 120, agentSession: agent('running', T0) })), 'worker が実装中（2分前から）');
  assert.equal(now(session('w1', { present: false }), task('1', { prTurn: 'checks' })), 'PR は bot・CI 待ち');
  assert.equal(now(session('w1', { phase: 'pr', phaseAt: NOW, agentSession: agent('running', T0) }), task('1', { prTurn: 'other-reviewer' })), 'worker が PR 中（たった今から）');
  assert.equal(now(session('w1', { present: false }), task('1', { prTurn: 'other-reviewer' })), 'PR は他の人のレビュー待ち');
  assert.equal(now(session('w1', { present: false })), 'worker はいません');
});

test('a parent merges its children, each by its own mark, with the child titles', () => {
  const a = entryOf(session('wa', { phases: [['implement', T0 + 100]] }), task('a'));
  const b = entryOf(session('wb', { waiting: waiting('gb', T0 + 150, 'diff'), agentSession: agent('done', T0 + 151) }), task('b'));
  const c = entryOf(session('wc', { phases: [['plan', T0 + 50]] }), task('c', { parked: { reason: 'other', since: stamp(T0) } }));
  const d = entryOf(session('wd', { phases: [['plan', T0 + 400]] }), task('d'));
  const m = workParentAway([
    { title: 'A', entry: a, mark: { left: T0 + 10 } },
    { title: 'B', entry: b, mark: { left: T0 + 10 } },
    // Left after its phase: nothing since.
    { title: 'C', entry: c, mark: { left: T0 + 60 } },
    // Never opened: it adds nothing, but it is counted in what the children wait on.
    { title: 'D', entry: d, mark: undefined },
    { title: 'E', entry: null, mark: { left: T0 + 10 }, mergedAt: T0 + 120 },
    { title: 'F', entry: null, mark: undefined, mergedAt: T0 + 120 },
  ], NOW);
  assert.deepEqual(plain(m.events.map(x => [x.title, x.kind, x.at])), [['A', 'phase', T0 + 100], ['E', 'pr', T0 + 120], ['B', 'gate', T0 + 150]]);
  assert.equal(m.now, '判定待ち 1 · 状態不明 2 · 置いている 1');
  assert.equal(workParentAway([{ title: 'D', entry: d, mark: undefined }], NOW), null);
});

test('a parent with only merged children that have no row says nothing when they merged before the child was left', () => {
  const kid = (mergedAt, mark = { left: T0 + 100 }) => ({ title: 'E', entry: null, mark, mergedAt });
  // Merged at or before `since`: no event, no last action, no counts, so no block.
  assert.equal(workParentAway([kid(T0 + 100), kid(T0 + 50)], NOW), null);
  const m = workParentAway([kid(T0 + 150)], NOW);
  assert.deepEqual(plain(m.events.map(x => [x.title, x.turn])), [['E', 'merged']]);
  assert.equal(m.now, '');
});
