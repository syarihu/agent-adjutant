/* The pure half of 「いまの仕事」's desktop notifications (src/ui/my-work-notify.js), run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-away.js', 'my-work-notify.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const { notifyNoticeRings, notifyWaitTag, notifyGateTag, notifyOwnNotifier, notifyRungHas, notifyRungAdd, notifyRungPrune, notifyCanRing, notifyAppHolds, notifyNotices, notifyNoticeContent, notifyPageRings, notifyPendingFinal, notifyGateMatch, notifyWaitMatch, notifyPrefs, notifyEffective, notifyQuiet, notifyEndEvents, notifyContent, notifyTrimMessage, notifyReviewContent } = ctx;

const T0 = Date.UTC(2026, 9, 1) / 1000;
const DEFAULTS = { waiting: true, done: false, failed: true, review: true };
const ALL = { waiting: true, done: true, failed: true, review: true };
const entry = (id, items, extra = {}) => ({ id, nwo: 'acme/widget', board: 'acme-widget', ref: id, task: { id, title: `Task ${id}` }, session: null, isHub: false, gates: [], items, ...extra });
const done = since => ({ kind: 'done', since });
const failed = since => ({ kind: 'failed', since });
const run = (entries, state, o = {}) => notifyEndEvents(entries, state, { prefs: ALL, openId: null, visible: true, okRepos: ['acme/widget'], ...o });
const kinds = r => plain(r.events.map(e => `${e.entry.id}:${e.kind}`));

test('the page rings waits and gates only with permission and the waiting toggle on', () => {
  assert.strictEqual(notifyPageRings('granted', { waiting: true }), true);
  assert.strictEqual(notifyPageRings('denied', { waiting: true }), false);
  assert.strictEqual(notifyPageRings('default', { waiting: true }), false);
  assert.strictEqual(notifyPageRings('granted', { waiting: false }), false);
  assert.strictEqual(notifyPageRings('granted', undefined), false);
});

test('the page can notify with permission alone, whatever the waiting toggle says', () => {
  assert.strictEqual(notifyCanRing('granted'), true);
  assert.strictEqual(notifyCanRing('denied'), false);
  assert.strictEqual(notifyCanRing('default'), false);
  assert.strictEqual(notifyCanRing(undefined), false);
  // Permission with 確認待ち off: it can notify, it does not ring waits.
  assert.strictEqual(notifyCanRing('granted') && notifyPageRings('granted', { waiting: false }), false);
});

test('a browser page is silent while the app is open, and the app is never silenced by itself', () => {
  assert.strictEqual(notifyAppHolds(false, true), true);
  assert.strictEqual(notifyAppHolds(false, false), false);
  assert.strictEqual(notifyAppHolds(true, true), false);
  assert.strictEqual(notifyAppHolds(true, false), false);
  // A header that was never read is not an open app.
  assert.strictEqual(notifyAppHolds(false, undefined), false);
  assert.strictEqual(notifyAppHolds(false, 'true'), false);
});

test('the notices the server hands over are read back, and anything else is no notice', () => {
  const raw = JSON.stringify([{ id: 'a-1', title: 'adjutant / widget', message: '確認待ち', nwo: 'acme/widget' }, { id: 'a-2', title: 't' }]);
  assert.deepStrictEqual(plain(notifyNotices(raw)), [
    { id: 'a-1', title: 'adjutant / widget', message: '確認待ち', tag: null },
    { id: 'a-2', title: 't', message: '', tag: null },
  ]);
  // The server writes non-ASCII as escapes: they read back as the text.
  assert.strictEqual(notifyNotices('[{"id":"x","title":"t","message":"\\u78ba\\u8a8d"}]')[0].message, '確認');
  for (const bad of ['', 'not json', '{}', 'null', '[1, "x", null]', '[{"id":"","title":"t"}]', '[{"id":"a"}]', '[{"id":1,"title":"t"}]']) {
    assert.deepStrictEqual(plain(notifyNotices(bad)), [], bad);
  }
});

test('a notice carries the tag the page rings its own event under, and an old one has none', () => {
  const [a, b] = notifyNotices(JSON.stringify([{ id: 'a', title: 't', tag: 'gate-acme/g7' }, { id: 'b', title: 't' }]));
  assert.strictEqual(a.tag, 'gate-acme/g7');
  assert.strictEqual(b.tag, null);
  assert.strictEqual(notifyNoticeContent(a).tag, 'gate-acme/g7');
  assert.strictEqual(notifyNoticeContent(b).tag, 'notice-b');
});

test('an event is rung once whichever of the page and the notice comes second', () => {
  const rung = new Map();
  const T = 1_000_000;
  assert.strictEqual(notifyRungHas(rung, 'gate-acme/g7', T), false);
  notifyRungAdd(rung, 'gate-acme/g7', T);
  assert.strictEqual(notifyRungHas(rung, 'gate-acme/g7', T + 1000), true);
  assert.strictEqual(notifyRungHas(rung, 'gate-acme/g8', T + 1000), false);
  // Other hubs' gates of the same id are other events.
  assert.strictEqual(notifyRungHas(rung, 'gate-beta/g7', T + 1000), false);
  // Forgotten after ten minutes.
  assert.strictEqual(notifyRungHas(rung, 'gate-acme/g7', T + 10 * 60 * 1000 + 1), false);
});

test('the set is bounded, dropping the oldest', () => {
  const rung = new Map();
  for (let i = 0; i < 250; i++) notifyRungAdd(rung, 'wait-a/' + i + '/1', 5000 + i);
  assert.strictEqual(rung.size, 200);
  assert.strictEqual(rung.has('wait-a/0/1'), false);
  assert.strictEqual(rung.has('wait-a/249/1'), true);
});

test('a wait is named by its row and moment alone, a gate keeps its hub', () => {
  assert.strictEqual(notifyWaitTag('row-1', 1700), 'wait-row-1/1700');
  assert.strictEqual(notifyGateTag('acme-widget', 'g7'), 'gate-acme-widget/g7');
  // One wait seen from two boards is one event; two hubs' gates with the same id are two.
  const rung = new Map();
  notifyRungAdd(rung, notifyWaitTag('row-1', 1700), 10);
  assert.strictEqual(notifyRungHas(rung, notifyWaitTag('row-1', 1700), 11), true);
  assert.strictEqual(notifyRungHas(rung, notifyWaitTag('row-1', 1701), 11), false);
  notifyRungAdd(rung, notifyGateTag('acme-widget', 'g7'), 10);
  assert.strictEqual(notifyRungHas(rung, notifyGateTag('acme-widget', 'g7'), 11), true);
  assert.strictEqual(notifyRungHas(rung, notifyGateTag('acme-parent', 'g7'), 11), false);
});

test('a notice for a wait or gate follows the waiting setting, any other is rung as before', () => {
  const tagged = tag => ({ id: 'a', title: 't', message: '', tag });
  for (const tag of ['wait-row-1/1700', 'gate-acme/g7']) {
    assert.strictEqual(notifyNoticeRings(tagged(tag), true), true, tag);
    assert.strictEqual(notifyNoticeRings(tagged(tag), false), false, tag);
  }
  // Mail and `adj notify` carry no such tag, or one of their own.
  for (const tag of [null, undefined, 'notice-a', 'mail-x']) {
    assert.strictEqual(notifyNoticeRings(tagged(tag), false), true, String(tag));
    assert.strictEqual(notifyNoticeRings(tagged(tag), true), true, String(tag));
  }
});

test('a board with a notifier of its own is not held back for the app, the others are', () => {
  const own = ['acme-cmd'];
  assert.strictEqual(notifyAppHolds(false, true, 'acme-cmd', own), false);
  assert.strictEqual(notifyAppHolds(false, true, 'acme-app', own), true);
  assert.strictEqual(notifyAppHolds(false, false, 'acme-app', own), false);
  assert.strictEqual(notifyAppHolds(true, true, 'acme-app', own), false);
  // A board served alone is "".
  assert.strictEqual(notifyAppHolds(false, true, '', ['']), false);
  assert.strictEqual(notifyAppHolds(false, true, '', []), true);
  // What belongs to no board is held while the app is open.
  assert.strictEqual(notifyAppHolds(false, true, null, []), true);
  assert.strictEqual(notifyAppHolds(false, true, null, own), true);
  assert.strictEqual(notifyAppHolds(false, true, undefined, undefined), true);
});

test('the boards with a notifier of their own are read from the header, and garbage is none', () => {
  assert.deepStrictEqual(plain(notifyOwnNotifier('["a","b"]')), ['a', 'b']);
  assert.deepStrictEqual(plain(notifyOwnNotifier('[""]')), ['']);
  assert.deepStrictEqual(plain(notifyOwnNotifier('["a",1,null]')), ['a']);
  for (const bad of ['', 'x', '{}', 'null', '3']) assert.deepStrictEqual(plain(notifyOwnNotifier(bad)), [], bad);
});

test('a notice shows its title and message, tagged by its id', () => {
  assert.deepStrictEqual(plain(notifyNoticeContent({ id: 'a-1', title: 'adjutant / widget', message: 'done' })),
    { title: 'adjutant / widget', body: 'done', tag: 'notice-a-1' });
});

test('the settings default to waiting, failed and review, and garbage is the default', () => {
  assert.deepStrictEqual(plain(notifyPrefs(undefined)), DEFAULTS);
  assert.deepStrictEqual(plain(notifyPrefs('x')), DEFAULTS);
  assert.deepStrictEqual(plain(notifyPrefs([true])), DEFAULTS);
  assert.deepStrictEqual(plain(notifyPrefs({ waiting: 'no', done: 1, failed: null })), DEFAULTS);
  assert.deepStrictEqual(plain(notifyPrefs({ waiting: false, done: true, failed: false, review: false })), { waiting: false, done: true, failed: false, review: false });
  assert.deepStrictEqual(plain(notifyPrefs({ waiting: false, done: true, failed: false, review: 'x' })), { waiting: false, done: true, failed: false, review: true });
});

test('settings saved before review existed get review on, and keep their other choices', () => {
  assert.deepStrictEqual(plain(notifyPrefs({ waiting: true, done: false, failed: true })), DEFAULTS);
  assert.strictEqual(notifyPrefs({ waiting: false, done: true, failed: false }).review, true);
});

test('with review requests turned off only the review kind is off, and the saved choice is left as it was', () => {
  const input = { waiting: true, done: false, failed: true, review: true };
  const off = notifyEffective(input, false);
  assert.deepStrictEqual(plain(off), { waiting: true, done: false, failed: true, review: false });
  assert.strictEqual(input.review, true);
  assert.notStrictEqual(off, input);

  const on = notifyEffective(input, true);
  assert.deepStrictEqual(plain(on), input);
  assert.notStrictEqual(on, input);

  const alreadyOff = { waiting: false, done: true, failed: false, review: false };
  assert.deepStrictEqual(plain(notifyEffective(alreadyOff, false)), alreadyOff);
  assert.deepStrictEqual(plain(notifyEffective(alreadyOff, true)), alreadyOff);
});

test('the first document rings nothing, and what it holds is learned', () => {
  const first = run([entry('a', [done(T0)]), entry('b', [failed(T0)])], null);
  assert.deepStrictEqual(plain(first.events), []);
  assert.equal(first.seen.size, 2);
  assert.deepStrictEqual(kinds(run([entry('a', [done(T0)]), entry('b', [failed(T0)])], first)), []);
});

test('an end rings once per since, and a new since rings again', () => {
  const a = [entry('a', [done(T0)])];
  const seen0 = run([], null);
  const r1 = run(a, seen0);
  assert.deepStrictEqual(kinds(r1), ['a:done']);
  assert.deepStrictEqual(kinds(run(a, r1)), []);
  assert.deepStrictEqual(kinds(run([entry('a', [done(T0 + 60)])], r1)), ['a:done']);
  assert.deepStrictEqual(kinds(run([entry('a', [failed(T0 + 90)])], r1)), ['a:failed']);
});

test('a kind that is off is silent, and turning it on later does not ring for what was there', () => {
  const seen0 = run([], null);
  const e = [entry('a', [done(T0)]), entry('b', [failed(T0)])];
  const off = run(e, seen0, { prefs: DEFAULTS });
  assert.deepStrictEqual(kinds(off), ['b:failed']);
  assert.deepStrictEqual(kinds(run(e, off, { prefs: ALL })), []);
  assert.deepStrictEqual(kinds(run([entry('a', [done(T0 + 5)])], off, { prefs: ALL })), ['a:done']);
});

test('a task that is parked is silent for every kind', () => {
  const task = { id: 'a', title: 'A', status: 'dispatched', parked: { reason: 'review', since: '20261001T000000Z' } };
  const r = run([entry('a', [failed(T0)], { task }), entry('b', [done(T0)], { task })], run([], null));
  assert.deepStrictEqual(kinds(r), []);
});

test('the row open on screen is silent while the page is visible, and rings while it is hidden', () => {
  const seen0 = run([], null);
  const e = [entry('a', [done(T0)])];
  assert.deepStrictEqual(kinds(run(e, seen0, { openId: 'a', visible: true })), []);
  assert.deepStrictEqual(kinds(run(e, seen0, { openId: 'a', visible: false })), ['a:done']);
  assert.deepStrictEqual(kinds(run(e, seen0, { openId: 'other', visible: true })), ['a:done']);
  assert.equal(notifyQuiet(e[0], 'a', true), true);
  assert.equal(notifyQuiet(e[0], 'a', false), false);
});

test('one event to an entry in a round, a failure first', () => {
  const r = run([entry('a', [done(T0), failed(T0)])], run([], null));
  assert.deepStrictEqual(kinds(r), ['a:failed']);
});

test('a hub, or an entry with a gate open, has no completion', () => {
  const sess = status => ({ id: 's', kind: 'worker', present: true, agentSession: { status, updatedAt: T0 } });
  const hub = { kind: 'hub', present: true, id: 'h', agentSession: { status: 'done', updatedAt: T0 } };
  const doc = { now: T0, repos: [{ nwo: 'acme/widget', rows: [
    { board: 'b', session: hub },
    { board: 'b', session: { ...sess('done'), waiting: { id: 'g', kind: 'plan', slug: 'b', openedAt: '20261001T000000Z' } }, task: { id: '1', title: 'One' } },
    { board: 'b', session: sess('done'), task: { id: '2', title: 'Two' } },
  ], hubSessions: [], turns: [] }] };
  const r = run(ctx.workEntries(doc), run([], null));
  assert.deepStrictEqual(kinds(r), ['b/2:done']);
});

test('the content says state, repository and branch first, then the request or the last message', () => {
  const e = entry('a', [], { session: { branch: 'feat/x', agentSession: { lastMessage: 'All done.\nSecond line' } } });
  assert.deepStrictEqual(plain(notifyContent('done', e)), { title: 'Task a', body: '完了 · widget · feat/x\nAll done.\nSecond line' });
  assert.deepStrictEqual(plain(notifyContent('permission', e, { request: '許可を求めています: Bash: make' })),
    { title: 'Task a', body: '入力待ち · widget · feat/x\n許可を求めています: Bash: make' });
  assert.deepStrictEqual(plain(notifyContent('gate', e, { gate: 'Plan review' })), { title: 'Task a', body: '確認待ち · widget · feat/x\nPlan review' });
  // Nothing missing is written as a gap.
  const bare = entry('b', [], { task: null, nwo: '' });
  assert.deepStrictEqual(plain(notifyContent('failed', bare, {})), { title: '', body: '失敗' });
  assert.equal(notifyContent('gate', bare, { gate: 'G' }).title, 'G');
});

test('a long last message is cut to a few lines and characters with an ellipsis', () => {
  assert.equal(notifyTrimMessage('a\nb\nc\nd'), 'a\nb\nc…');
  assert.equal(notifyTrimMessage('a\n\n b '), 'a\nb');
  const long = notifyTrimMessage('x'.repeat(500));
  assert.equal(long.length, 201);
  assert.ok(long.endsWith('…'));
  assert.equal(notifyTrimMessage(undefined), '');
  assert.equal(notifyTrimMessage('# Done\n- **a**\n- `b`'), 'Done\na\nb');
});

test('an entry that leaves the document for a round and comes back does not ring again', () => {
  const e = [entry('a', [done(T0)])];
  const r1 = run(e, run([], null));
  assert.deepStrictEqual(kinds(r1), ['a:done']);
  const gone = run([], r1);
  assert.deepStrictEqual(kinds(run(e, gone)), []);
});

test('a turn that ended under an open gate is not a new completion when the gate is answered', () => {
  const sess = { id: 's', kind: 'worker', present: true, agentSession: { status: 'done', updatedAt: T0 } };
  const gate = { kind: 'gate', gate: 'plan', title: 'P', since: T0 + 1, key: 'b/g' };
  const gated = entry('a', [gate], { session: sess, gates: [gate] });
  const r1 = run([gated], run([], null));
  assert.deepStrictEqual(kinds(r1), []);
  const answered = entry('a', [done(T0)], { session: sess });
  assert.deepStrictEqual(kinds(run([answered], r1)), []);
});

test('a gate states its kind in the first line', () => {
  const e = entry('a', [], { session: { branch: 'x' } });
  assert.equal(notifyContent('gate', e, { gate: 'G', label: '計画' }).body, '確認待ち（計画） · widget · x\nG');
});

test('a gate or a wait is matched to its entry by key, task, session or ledger row', () => {
  const e = entry('t', [], { gates: [{ key: 'b/g1' }], session: { id: 's1', agentSession: { sessionId: 'L1' } }, board: 'b' });
  assert.equal(notifyGateMatch(e, 'b', 'g1', ''), true);
  assert.equal(notifyGateMatch(e, 'b', 'g2', 't'), true);
  assert.equal(notifyGateMatch(e, 'c', 'g2', 't'), false);
  assert.equal(notifyWaitMatch(e, 'b', 's1', 'x'), true);
  assert.equal(notifyWaitMatch(e, 'b', 's2', 'L1'), true);
  assert.equal(notifyWaitMatch(e, 'c', 's1', 'L1'), false);
  assert.equal(notifyWaitMatch(e, 'b', 's2', 'L2'), false);
});

test('a repository missing from the first document learns its history silently, and rings for what is new after', () => {
  const r1 = run([], null, { okRepos: [] });
  const old = [entry('a', [failed(T0)]), entry('b', [done(T0)])];
  const r2 = run(old, r1);
  assert.deepStrictEqual(kinds(r2), []);
  const r3 = run([...old, entry('c', [failed(T0 + 50)])], r2);
  assert.deepStrictEqual(kinds(r3), ['c:failed']);
});

test('a repository that carries an error in a document only learns', () => {
  const r1 = run([entry('a', [done(T0)])], null);
  const r2 = run([entry('a', [done(T0 + 5)])], r1, { okRepos: [] });
  assert.deepStrictEqual(kinds(r2), []);
  assert.deepStrictEqual(kinds(run([entry('a', [done(T0 + 5)])], r2)), []);
});

test('an end with no time is neither learned nor rung', () => {
  const r = run([entry('a', [done(null)])], run([], null));
  assert.deepStrictEqual(kinds(r), []);
  assert.equal(r.seen.size, 0);
});

test('a session that flaps absent does not ring its old end when it returns', () => {
  const agent = { status: 'done', updatedAt: T0 };
  const away = entry('a', [], { session: { id: 's', present: false, agentSession: agent } });
  const r1 = run([away], run([], null));
  const back = entry('a', [done(T0)], { session: { id: 's', present: true, agentSession: agent } });
  assert.deepStrictEqual(kinds(run([back], r1)), []);
});

test('the last message is the body only when it belongs to the turn that ended', () => {
  const at = (said, extra = {}) => entry('a', [], { session: { agentSession: { updatedAt: T0, lastMessage: 'Old news', lastMessageAt: said, ...extra } } });
  assert.equal(notifyContent('done', at(T0 - 600)).body, '完了 · widget');
  assert.equal(notifyContent('done', at(T0 - 30)).body, '完了 · widget\nOld news');
  assert.equal(notifyContent('done', at(undefined)).body, '完了 · widget\nOld news');
});

test('a queued item is final once the document was fetched after it was queued, or after it has waited long enough', () => {
  const MAX = 15000;
  assert.equal(notifyPendingFinal(2000, 1000, 3000, MAX), false);
  assert.equal(notifyPendingFinal(500, 1000, 3000, MAX), true);
  assert.equal(notifyPendingFinal(2000, null, 3000, MAX), false);
  assert.equal(notifyPendingFinal(2000, null, 17000, MAX), true);
  assert.equal(notifyPendingFinal(2000, 1000, 17000, MAX), true);
});

const reviewRec = (n, extra = {}) => ({ id: `acme/widget#${n}`, repo: 'Acme/Widget', number: n, title: `Fix ${n}`, author: 'octo', ...extra });

test('nothing arrived is no notification', () => {
  assert.strictEqual(notifyReviewContent([], [reviewRec(1)]), null);
  assert.strictEqual(notifyReviewContent(undefined, []), null);
});

test('one request is titled with the PR title and says repository, number and author', () => {
  assert.deepStrictEqual(plain(notifyReviewContent(['acme/widget#7'], [reviewRec(7), reviewRec(8)])),
    { title: 'Fix 7', body: 'レビュー依頼 · Widget · #7 · @octo' });
  assert.equal(notifyReviewContent(['acme/widget#7'], [reviewRec(7, { author: null })]).body, 'レビュー依頼 · Widget · #7');
});

test('several requests say how many, with up to three titles, and an id with no record still counts', () => {
  const recs = [1, 2, 3, 4].map(n => reviewRec(n));
  const ids = recs.map(r => r.id);
  assert.deepStrictEqual(plain(notifyReviewContent(ids.slice(0, 3), recs)), { title: 'レビュー依頼 3 件', body: 'Fix 1\nFix 2\nFix 3' });
  assert.equal(notifyReviewContent(ids, recs).body, 'Fix 1\nFix 2\nFix 3');
  assert.equal(notifyReviewContent(['acme/widget#1', 'acme/widget#99'], recs).title, 'レビュー依頼 2 件');
  assert.equal(notifyReviewContent(['acme/widget#99'], recs).title, 'レビュー依頼 1 件');
});
