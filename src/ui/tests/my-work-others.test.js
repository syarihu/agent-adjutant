/* The pure half of 「AI 下読み」 (src/ui/my-work-others.js), run with `node --test`. The page's scripts share one global
   scope, so they are loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const { othersItems, othersActedAt, othersSeenClass, othersNewIds, othersSize, othersStale, othersStaleMinutes, othersGroups, othersRepoChips,
  othersEffectiveRepo, othersStateLine, othersStateOf, othersHubView, othersSig, othersOutOfReview, othersRequestedLabel, othersAiView } = ctx;

const stamp = secs => {
  const d = new Date(secs * 1000);
  const p = n => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}T${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}Z`;
};
const T0 = Date.UTC(2026, 9, 1) / 1000;
const rec = (n, extra = {}) => ({ id: `acme/widget#${n}`, repo: 'Acme/Widget', number: n, title: `PR ${n}`, state: 'requested', registered: true,
  requestedAt: stamp(T0), additions: 10, deletions: 5, events: { requested: stamp(T0), rerequested: null, aiReady: null, pushed: null }, ...extra });

test('the items of a record are its events that have a time, and a record waiting on its author or done has none', () => {
  const r = rec(1, { events: { requested: stamp(T0), rerequested: stamp(T0 + 5), aiReady: null, pushed: stamp(T0 + 9) } });
  assert.deepStrictEqual(plain(othersItems(r)), [{ kind: 'requested', since: T0 }, { kind: 'rerequested', since: T0 + 5 }, { kind: 'pushed', since: T0 + 9 }]);
  for (const state of ['waiting-on-author', 'done', 'something-new']) assert.deepStrictEqual(plain(othersItems(rec(2, { state }))), []);
  assert.deepStrictEqual(plain(othersItems(rec(5, { events: null }))), []);
});

test('a record is new after an event, not once read at or after it, and new again after a later event', () => {
  const r = rec(1);
  assert.equal(othersSeenClass(r, {}), 'new');
  assert.equal(othersSeenClass(r, { read: T0 - 1 }), 'new');
  assert.equal(othersSeenClass(r, { read: T0 }), null);
  assert.equal(othersSeenClass(r, { read: T0 + 100 }), null);
  const pushed = rec(1, { state: 'pushed', events: { ...r.events, pushed: stamp(T0 + 200) } });
  assert.equal(othersSeenClass(pushed, { read: T0 + 100 }), 'new');
  const again = rec(1, { events: { ...r.events, rerequested: stamp(T0 + 300) } });
  assert.equal(othersSeenClass(again, { read: T0 + 250 }), 'new');
  assert.equal(othersSeenClass(again, { read: T0 + 300 }), null);
});

test('starting the AI 下読み reads the row, and the read-through finishing makes it new again', () => {
  const started = rec(1, { state: 'ai-reading', ai: { status: 'running', startedAt: stamp(T0 + 10) } });
  assert.equal(othersActedAt(started), T0 + 10);
  assert.equal(othersSeenClass(started, {}), null);
  const ready = rec(1, { state: 'ai-ready', ai: { status: 'ready', startedAt: stamp(T0 + 10), finishedAt: stamp(T0 + 60) },
    events: { requested: stamp(T0), rerequested: null, aiReady: stamp(T0 + 60), pushed: null } });
  assert.equal(othersSeenClass(ready, {}), 'new');
  assert.equal(othersSeenClass(ready, { read: T0 + 60 }), null);
  assert.equal(othersActedAt(rec(2)), -Infinity);
});

test('a waiting-on-author or done record is never new, and the new ids ignore any filter', () => {
  const rows = [rec(1), rec(2, { state: 'waiting-on-author' }), rec(3, { state: 'done' }), rec(4, { repo: 'Other/Repo', id: 'other/repo#4' })];
  assert.deepStrictEqual(plain(othersNewIds(rows, {})), ['acme/widget#1', 'other/repo#4']);
  assert.deepStrictEqual(plain(othersNewIds(rows, { 'acme/widget#1': { read: T0 } })), ['other/repo#4']);
});

test('the size is S below 200 lines, M below 1000, and L from there', () => {
  const size = n => othersSize({ additions: n, deletions: 0 });
  assert.deepStrictEqual([size(199), size(200), size(999), size(1000)], ['S', 'M', 'M', 'L']);
  assert.equal(othersSize({ additions: 150, deletions: 60 }), 'M');
  assert.equal(othersSize({}), 'S');
});

test('a list is stale after exactly 30 minutes, and one never synced is not', () => {
  const ls = { at: stamp(T0) };
  assert.equal(othersStale(ls, T0 + 1800), false);
  assert.equal(othersStale(ls, T0 + 1801), true);
  assert.equal(othersStale(null, T0 + 99999), false);
  assert.equal(othersStale({ at: null }, T0 + 99999), false);
  assert.equal(othersStaleMinutes(ls, T0 + 1861), 31);
  assert.equal(othersStaleMinutes(null, T0), 0);
});

test('groups are by repository with case ignored, labelled as GitHub spells it, and flag one that is not registered', () => {
  const rows = [rec(1, { repo: 'zeta/App', id: 'zeta/app#1' }), rec(2, { repo: 'Acme/Widget' }), rec(3, { repo: 'acme/widget', id: 'acme/widget#3', registered: false })];
  const groups = othersGroups(rows);
  assert.deepStrictEqual(plain(groups.map(g => [g.key, g.label, g.unregistered, g.rows.length])),
    [['acme/widget', 'Acme/Widget', true, 2], ['zeta/app', 'zeta/App', false, 1]]);
});

test('rows are oldest request first, none last, done after all of them, then by number', () => {
  const rows = [
    rec(5, { state: 'done', requestedAt: stamp(T0 - 999) }),
    rec(4, { requestedAt: null }),
    rec(3, { requestedAt: stamp(T0 + 10) }),
    rec(2, { requestedAt: stamp(T0) }),
    rec(1, { requestedAt: stamp(T0) }),
  ];
  assert.deepStrictEqual(plain(othersGroups(rows)[0].rows.map(r => r.number)), [1, 2, 3, 4, 5]);
});

test('the filter narrows to one repository, and a repository with no record is すべて', () => {
  const rows = [rec(1), rec(2, { repo: 'Zeta/App', id: 'zeta/app#2' })];
  assert.deepStrictEqual(plain(othersGroups(rows, 'zeta/app').map(g => g.key)), ['zeta/app']);
  assert.deepStrictEqual(plain(othersGroups(rows, 'gone/repo').map(g => g.key)), ['acme/widget', 'zeta/app']);
  assert.equal(othersEffectiveRepo(rows, 'gone/repo'), '');
  assert.equal(othersEffectiveRepo(rows, 'zeta/app'), 'zeta/app');
  assert.deepStrictEqual(plain(othersRepoChips(rows)), [{ key: 'acme/widget', label: 'Acme/Widget', count: 1 }, { key: 'zeta/app', label: 'Zeta/App', count: 1 }]);
});

test('what a row says of its state, for every state', () => {
  const line = (state, extra) => plain(othersStateLine(rec(1, { state, ...extra })));
  assert.deepStrictEqual(line('requested'), { cls: '', text: 'あなたのレビュー待ち' });
  assert.equal(line('requested', { rerequest: true }).text, 'あなたのレビュー待ち · レビューを頼み直されました');
  assert.equal(line('ai-reading', { ai: { status: 'running' } }).text, 'AI 下読み中');
  assert.equal(line('ai-reading', { ai: { status: 'queued' } }).text, 'AI 下読みの順番待ち');
  assert.equal(line('ai-ready').text, '下読み済み');
  assert.equal(line('ai-ready', { ai: { status: 'ready', counts: { attention: 2, check: 0, drafts: 3 } } }).text, '下読み済み · 要注意 2 · 確認 0 · 下書き 3');
  assert.equal(line('pushed', { commitsSinceReview: 3 }).text, '前回のレビューから +3 コミット');
  assert.equal(line('pushed', { commitsSinceReview: 0 }).text, '前回のレビューから +0 コミット');
  assert.equal(line('pushed', { commitsSinceReview: null }).text, '前回のレビューのあとに push されました');
  assert.equal(line('pushed', { commitsSinceReview: 1, rerequest: true }).text, '前回のレビューから +1 コミット · レビューを頼み直されました');
  assert.deepStrictEqual(line('waiting-on-author', { myReview: { state: 'changes-requested' } }), { cls: 'dim', text: '作者の対応待ち — 変更を依頼しました' });
  assert.equal(line('waiting-on-author', { myReview: { state: 'commented' } }).text, '作者の対応待ち — コメントしました');
  for (const [reason, word] of [['approved', '承認しました'], ['merged', 'マージされました'], ['closed', '閉じられました'], ['withdrawn', '依頼が取り下げられました']]) {
    assert.deepStrictEqual(line('done', { doneReason: reason }), { cls: 'dim', text: `終わった — ${word} · 1日で消えます` });
  }
});

test('a state this page does not know is shown as it came and never as a request', () => {
  assert.deepStrictEqual(plain(othersStateOf(rec(1, { state: 'brand-new' }))), ['help', 'brand-new', 'ov-dim']);
  assert.notEqual(othersStateOf(rec(1, { state: 'brand-new' }))[1], othersStateOf(rec(1))[1]);
  assert.deepStrictEqual(plain(othersStateLine(rec(1, { state: 'brand-new' }))), { cls: 'dim', text: 'brand-new' });
  assert.strictEqual(othersOutOfReview(rec(1)), null);
});

test('the time since the request is the words of the list, and nothing without one', () => {
  assert.equal(othersRequestedLabel(rec(1), T0 + 5 * 60), '依頼 5分前');
  assert.equal(othersRequestedLabel(rec(1, { requestedAt: null }), T0), '');
});

test('the AI 下読み of a record: what it says and the one button it offers, for every status', () => {
  const view = (extra, state = 'requested', records) => plain(othersAiView(rec(1, { state, ...extra }), records));
  assert.deepStrictEqual(view({}), { kind: 'none', text: '', action: 'AI 下読み' });
  assert.equal(view({}, 'pushed').action, 'AI 下読み');
  // A PR waiting on its author, or done, is offered nothing.
  assert.equal(view({}, 'waiting-on-author').action, null);
  assert.equal(view({}, 'done').action, null);
  assert.deepStrictEqual(view({ ai: { status: 'running' } }, 'ai-reading'), { kind: 'running', text: 'AI 下読み中', action: null });
  assert.deepStrictEqual(view({ ai: { status: 'queued' } }, 'ai-reading'), { kind: 'queued', text: 'AI 下読みの順番待ち', action: null });
  assert.deepStrictEqual(
    view({ ai: { status: 'ready', counts: { attention: 2, check: 0, drafts: 3 } } }, 'ai-ready'),
    { kind: 'done', text: '下読み済み · 要注意 2 · 確認 0 · 下書き 3', action: null });
  assert.deepStrictEqual(
    view({ ai: { status: 'ready', stale: true, sha: '0123456789abcdef' } }, 'ai-ready'),
    { kind: 'stale', text: '下読み済み（前の push 時点 0123456）', action: '下読みし直す' });
  assert.deepStrictEqual(
    view({ ai: { status: 'failed', error: 'the reader exited' } }),
    { kind: 'failed', text: 'AI 下読みに失敗しました: the reader exited', action: 'もう一度 AI 下読み' });
  assert.equal(view({ ai: { status: 'failed' } }).text, 'AI 下読みに失敗しました');
  // A status from a newer server says and offers nothing.
  assert.deepStrictEqual(view({ ai: { status: 'brand-new' } }), { kind: 'none', text: '', action: null });
});

test('a read running for more than 30 minutes is a hung reader and offers the button again', () => {
  const launched = T0;
  const running = rec(1, { state: 'ai-reading', ai: { status: 'running', launchedAt: stamp(launched) } });
  const view = now => plain(othersAiView(running, [running], now));
  assert.deepStrictEqual(view(launched + 1800), { kind: 'running', text: 'AI 下読み中', action: null });
  assert.deepStrictEqual(view(launched + 1801), { kind: 'hung', text: 'AI 下読み中（30 分以上たっています）', action: 'もう一度 AI 下読み' });
  // Without a clock, or a launch time, nothing is known to be old.
  assert.equal(plain(othersAiView(running, [running])).kind, 'running');
  assert.equal(plain(othersAiView(rec(2, { state: 'ai-reading', ai: { status: 'running' } }), [], T0 + 99999)).kind, 'running');
  // The row says it too.
  assert.equal(plain(othersStateLine(running, [running], launched + 1801)).text, 'AI 下読み中（30 分以上たっています）');
});

test('a queued read says its place, by when it was asked and then by id', () => {
  const q = (n, at) => rec(n, { state: 'ai-reading', ai: { status: 'queued', startedAt: at } });
  const records = [q(3, '20261001T000200Z'), q(1, '20261001T000300Z'), q(2, '20261001T000200Z'), rec(9, { ai: { status: 'running' } })];
  const text = r => othersAiView(r, records).text;
  assert.equal(text(records[2]), 'AI 下読みの順番待ち（1 番目）');
  assert.equal(text(records[0]), 'AI 下読みの順番待ち（2 番目）');
  assert.equal(text(records[1]), 'AI 下読みの順番待ち（3 番目）');
  assert.equal(plain(othersStateLine(records[1], records)).text, 'AI 下読みの順番待ち（3 番目）');
});

test('the AI 下読み shows beside the state of a PR that is pushed or requested, and the state line keeps the rest', () => {
  const pushed = rec(1, { state: 'pushed', commitsSinceReview: 2, ai: { status: 'ready', stale: true, sha: 'abcdef012' } });
  assert.equal(plain(othersStateLine(pushed)).text, '前回のレビューから +2 コミット · 下読み済み（前の push 時点 abcdef0）');
  const failed = rec(2, { ai: { status: 'failed', error: 'boom' } });
  assert.equal(plain(othersStateLine(failed)).text, 'あなたのレビュー待ち · AI 下読みに失敗しました: boom');
});

test('the hub row: absent and stopped say AI 下読み is unavailable, and running counts the records', () => {
  const records = [rec(1, { ai: { status: 'running' } }), rec(2, { ai: { status: 'queued' } }), rec(3, { ai: { status: 'running' } }), rec(4)];
  const absent = plain(othersHubView(undefined, records));
  assert.deepStrictEqual([absent.state, absent.running, absent.max, absent.startable], ['stopped', 2, 3, false]);
  assert.match(absent.why, /まだありません。AI 下読みは使えません（同期と一覧はそのまま使えます）/);
  const stopped = plain(othersHubView({ state: 'stopped', startable: true, max: 5 }, records));
  assert.deepStrictEqual([stopped.state, stopped.max, stopped.startable], ['stopped', 5, true]);
  assert.match(stopped.why, /hub が止まっています。AI 下読みは使えません/);
  const up = plain(othersHubView({ state: 'working' }, records));
  assert.deepStrictEqual([up.state, up.why, up.max], ['working', '', 3]);
});

test('the hub row takes its numbers from the document, and a hub that is up has no reason to give', () => {
  const records = [rec(1, { ai: { status: 'running' } })];
  const running = plain(othersHubView({ state: 'running', slug: 'review', running: 2, queued: 1, max: 3, startable: false }, records));
  assert.deepStrictEqual(running, { state: 'running', running: 2, queued: 1, max: 3, startable: false, canRead: true, why: '' });
  const stopped = plain(othersHubView({ state: 'stopped', running: 0, queued: 4, max: 3, startable: true }, records));
  assert.deepStrictEqual([stopped.state, stopped.running, stopped.queued, stopped.startable], ['stopped', 0, 4, true]);
  assert.match(stopped.why, /hub が止まっています/);
  // A hub that is up but cannot open a reader (the terminal is not tmux) cannot be asked, and says why.
  const noReader = plain(othersHubView({ state: 'running', canRead: false }, records));
  assert.deepStrictEqual([noReader.state, noReader.canRead], ['running', false]);
  assert.match(noReader.why, /tmux/);
  assert.equal(plain(othersHubView({ state: 'stopped', canRead: true }, records)).canRead, false);
  assert.equal(plain(othersHubView({ state: 'running', canRead: true }, records)).why, '');
  // An older server says nothing of it, and a running hub is taken to be able.
  assert.equal(plain(othersHubView({ state: 'running' }, records)).canRead, true);
  // A hub that is up cannot be started again.
  assert.equal(plain(othersHubView({ state: 'running', startable: true }, records)).startable, false);
});

test('the signature changes with what is drawn and with the minute, and not with the seconds', () => {
  const doc = { now: T0 + 10, lastSync: { at: stamp(T0) }, records: [rec(1)] };
  const sig = othersSig(doc, T0 + 10);
  assert.equal(othersSig(doc, T0 + 20), sig);
  assert.notEqual(othersSig(doc, T0 + 70), sig);
  assert.notEqual(othersSig({ ...doc, records: [rec(1, { title: 'other' })] }, T0 + 10), sig);
  // A read-through that went stale, and the hub coming up, redraw.
  const read = stale => ({ ...doc, records: [rec(1, { ai: { status: 'ready', sha: 'a', stale } })] });
  assert.notEqual(othersSig(read(true), T0 + 10), othersSig(read(false), T0 + 10));
  assert.notEqual(othersSig({ ...doc, hub: { state: 'running' } }, T0 + 10), othersSig({ ...doc, hub: { state: 'stopped' } }, T0 + 10));
  // The stale flag is part of it, so the banner appears on time.
  assert.notEqual(othersSig(doc, T0 + 1801), othersSig(doc, T0 + 1799));
});
