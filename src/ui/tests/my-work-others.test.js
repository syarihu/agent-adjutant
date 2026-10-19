/* The pure half of 「AI 下読み」 (src/ui/my-work-others.js), run with `node --test`. The page's scripts share one global
   scope, so they are loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const othersWaiting = vm.runInContext('othersWaiting', ctx);
const { othersItems, othersActedAt, othersSeenClass, othersNewIds, othersSize, othersStale, othersSyncWords, othersHubStopText, othersGroups, othersRepoChips,
  othersEffectiveRepo, othersStateLine, othersStateOf, othersHubView, othersSig, othersOutOfReview, othersRequestedLabel, othersAiView,
  othersOutOfReviewLine, othersReadMode, othersReadAllPatches, othersAiStage, othersEffectiveStage, othersStageChips, othersHidesDone, othersShown } = ctx;

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

test('the rail\'s count: only the records in a state that waits on the person, in the repositories kept', () => {
  const records = [rec(1), rec(2, { state: 'ai-reading' }), rec(3, { state: 'ai-ready' }), rec(4, { state: 'pushed' }),
    rec(5, { state: 'waiting-on-author' }), rec(6, { state: 'done' }), rec(7, { repo: 'other/app' })];
  assert.deepStrictEqual(othersWaiting(records).map(r => r.number), [1, 2, 3, 4, 7]);
  assert.deepStrictEqual(othersWaiting(records, repo => repo === 'other/app').map(r => r.number), [7]);
  assert.deepStrictEqual(plain(othersWaiting(null)), []);
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

test('putting drafts into a pending review is acting on the row: it leaves 新着 until something newer happens', () => {
  const base = rec(1, { state: 'ai-ready', events: { requested: stamp(T0), rerequested: null, aiReady: stamp(T0 + 60), pushed: null }, ai: { status: 'ready', startedAt: stamp(T0 + 10), finishedAt: stamp(T0 + 60) } });
  assert.equal(othersSeenClass(base, {}), 'new');
  const pushed = { ...base, pendingReviewAt: stamp(T0 + 100) };
  assert.equal(othersActedAt(pushed), T0 + 100);
  assert.equal(othersSeenClass(pushed, {}), null);
  // A push of the PR after it is news again.
  assert.equal(othersSeenClass({ ...pushed, state: 'pushed', events: { ...base.events, pushed: stamp(T0 + 200) } }, {}), 'new');
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
});

test('the sync button says it is syncing, that there was no sync, or how long ago the last one was', () => {
  const ls = { at: stamp(T0) };
  assert.equal(othersSyncWords(ls, T0 + 60, true), '同期中…');
  assert.equal(othersSyncWords(null, T0, true), '同期中…');
  assert.equal(othersSyncWords(null, T0, false), 'まだ同期していません');
  assert.equal(othersSyncWords({ at: null }, T0, false), 'まだ同期していません');
  assert.match(othersSyncWords(ls, T0 + 2 * 3600, false), /^同期・最終同期: 2時間前$/);
  assert.equal(othersSyncWords(ls, T0 + 10, false), '同期・最終同期: たった今');
});

test('the confirmation of stopping the hub names the readers that keep running and the reads that wait, and only those there are', () => {
  const some = plain(othersHubStopText({ running: 2, queued: 1 }));
  assert.match(some.lead, /ペインを閉じます/);
  assert.equal(some.warnings.length, 2);
  assert.match(some.warnings[0], /^AI 下読み中の worker 2 件は止まりません/);
  assert.match(some.warnings[1], /^順番待ちの 1 件は、hub をもう一度起動するまで始まりません/);
  assert.match(some.note, /下読み・下書き・順番待ちは残ります/);
  assert.equal(plain(othersHubStopText({ running: 0, queued: 3 })).warnings.length, 1);
  assert.match(plain(othersHubStopText({ running: 0, queued: 3 })).warnings[0], /^順番待ちの 3 件/);
  assert.match(plain(othersHubStopText({ running: 1, queued: 0 })).warnings[0], /worker 1 件/);
  assert.deepStrictEqual(plain(othersHubStopText({ running: 0, queued: 0 })).warnings, []);
});

test('the hub can be stopped from the page only while it runs and the server says it can', () => {
  assert.equal(plain(othersHubView({ state: 'running', stoppable: true }, [])).stoppable, true);
  assert.equal(plain(othersHubView({ state: 'running' }, [])).stoppable, false);
  assert.equal(plain(othersHubView({ state: 'running', stoppable: false }, [])).stoppable, false);
  assert.equal(plain(othersHubView({ state: 'stopped', stoppable: true }, [])).stoppable, false);
  assert.equal(plain(othersHubView(undefined, [])).stoppable, false);
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
  // A reader that was closed says so, and not the error that closing left.
  assert.deepStrictEqual(
    view({ ai: { status: 'failed', error: 'the reader was closed', readerClosedAt: '20261001T000009Z' } }),
    { kind: 'failed', text: 'AI 下読みを閉じました（途中で止めました）', action: 'もう一度 AI 下読み' });
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
  assert.deepStrictEqual(running, { state: 'running', running: 2, queued: 1, max: 3, startable: false, stoppable: false, canRead: true, why: '' });
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

/* ── a PR whose change is too large to read ── */

const file = (path, additions, deletions = 0, change = 'modified') => ({ path, additions, deletions, change });

test('a PR is out of review when one file is strictly over 1,500 lines, and the files over are listed largest first', () => {
  const big = rec(1, { files: [file('src/small.rs', 100, 50), file('src/b.rs', 1000, 501), file('src/a.rs', 1500, 100), file('src/c.rs', 1000, 501)] });
  assert.deepStrictEqual(plain(othersOutOfReview(big)), {
    limit: 1500,
    files: [{ path: 'src/a.rs', additions: 1500, deletions: 100 }, { path: 'src/b.rs', additions: 1000, deletions: 501 }, { path: 'src/c.rs', additions: 1000, deletions: 501 }],
  });
  // Exactly 1,500 is not out of review; 1,501 is.
  assert.strictEqual(othersOutOfReview(rec(1, { files: [file('a.rs', 1000, 500)] })), null);
  assert.strictEqual(plain(othersOutOfReview(rec(1, { files: [file('a.rs', 1000, 501)] }))).files.length, 1);
  assert.strictEqual(othersOutOfReview(rec(1, { files: [] })), null);
  assert.strictEqual(othersOutOfReview(rec(1, { files: undefined })), null);
});

test('a file deleted whole, and lock files, snapshots and vendored code, are not counted; nor is the total of the PR', () => {
  const files = [file('old.rs', 0, 5000, 'deleted'), file('Cargo.lock', 4000, 10), file('src/__snapshots__/x.snap', 3000, 0), file('vendor/lib/x.js', 2000, 0)];
  assert.strictEqual(othersOutOfReview(rec(1, { files })), null);
  // Many files, none over the limit, however large the PR is.
  const many = Array.from({ length: 10 }, (_, i) => file(`src/f${i}.rs`, 1000, 400));
  assert.strictEqual(othersOutOfReview(rec(1, { files: many, additions: 10000, deletions: 4000 })), null);
});

test('the row\'s line names the largest file, and how many more are over', () => {
  const one = othersOutOfReview(rec(1, { files: [file('src/a.rs', 1200, 400)] }));
  assert.strictEqual(othersOutOfReviewLine(one), 'レビュー対象外 · src/a.rs（+1200 −400）');
  const three = othersOutOfReview(rec(1, { files: [file('src/a.rs', 1200, 400), file('src/b.rs', 2000, 0), file('src/c.rs', 1600, 0)] }));
  assert.strictEqual(othersOutOfReviewLine(three), 'レビュー対象外 · src/b.rs（+2000 −0） ほか 2 件');
  assert.strictEqual(othersOutOfReviewLine(null), '');
});

test('a split proposal says its own words and asks for another proposal', () => {
  const view = (ai, extra = {}, records) => plain(othersAiView(rec(1, { state: 'requested', files: [file('src/big.rs', 2000)], ai: { mode: 'split', ...ai }, ...extra }), records));
  assert.deepStrictEqual(view({ status: 'queued' }, { state: 'ai-reading' }), { kind: 'queued', text: '分割案の順番待ち', action: null, mode: 'split' });
  assert.equal(view({ status: 'queued', startedAt: stamp(T0) }, { state: 'ai-reading' }, [rec(1, { ai: { status: 'queued', startedAt: stamp(T0) } })]).text, '分割案の順番待ち（1 番目）');
  assert.deepStrictEqual(view({ status: 'running' }, { state: 'ai-reading' }), { kind: 'running', text: '分割案を作成中', action: null, mode: 'split' });
  const hung = plain(othersAiView(rec(1, { state: 'ai-reading', files: [file('src/big.rs', 2000)], ai: { mode: 'split', status: 'running', launchedAt: stamp(T0) } }), [], T0 + 1801));
  assert.deepStrictEqual(hung, { kind: 'hung', text: '分割案を作成中（30 分以上たっています）', action: 'もう一度 分割案', mode: 'split' });
  assert.deepStrictEqual(view({ status: 'ready', sha: 'a'.repeat(40) }, { state: 'ai-ready' }), { kind: 'done', text: '分割案あり', action: null, mode: 'split' });
  assert.deepStrictEqual(view({ status: 'ready', sha: 'abcdef0123', stale: true }, { state: 'ai-ready' }),
    { kind: 'stale', text: '分割案あり（前の push 時点 abcdef0）', action: '分割案を作り直す', mode: 'split' });
  assert.deepStrictEqual(view({ status: 'failed', error: 'boom' }), { kind: 'failed', text: '分割案の作成に失敗しました: boom', action: 'もう一度 分割案', mode: 'split' });
  // A read-through carries no mode, and the mode of a record is read or split.
  assert.ok(!('mode' in plain(othersAiView(rec(1, { ai: { status: 'running' }, state: 'ai-reading' })))));
  assert.deepStrictEqual([othersReadMode(rec(1)), othersReadMode(rec(1, { ai: { status: 'ready' } })), othersReadMode(rec(1, { ai: { status: 'ready', mode: 'split' } }))], ['read', 'read', 'split']);
});

test('the signature changes when only the files change', () => {
  const doc = files => ({ now: T0 + 10, lastSync: { at: stamp(T0) }, records: [rec(1, { files })] });
  assert.notEqual(othersSig(doc([file('a.rs', 100)]), T0 + 10), othersSig(doc([file('a.rs', 2000)]), T0 + 10));
});

/* The row is drawn by my-work-others-view.js, which touches the page when it is loaded: its function is cut out and given what it reads. */
function rowOf(rec, { canRead = true, asking = false } = {}) {
  const src = fs.readFileSync(path.join(__dirname, '..', 'my-work-others-view.js'), 'utf8');
  const m = src.match(/^function othersRowHtml[\s\S]*?\n\}/m);
  assert.ok(m);
  Object.assign(ctx, { others: { asking: new Set(asking ? [rec.id] : []) }, nav: { task: null }, PR_REF: 'pr:', OTHERS_SYNC_WHY: 'why', othersGlyphHtml: () => '' });
  vm.runInContext(m[0], ctx);
  return ctx.othersRowHtml(rec, false, { canRead, why: 'hub is down' }, T0 + 60, [rec]);
}

test('the row of a PR out of review says so, offers 分割案 and not AI 下読み', () => {
  const big = rec(1, { files: [file('src/a.rs', 1200, 400), file('src/b.rs', 2000, 0)] });
  const html = rowOf(big);
  assert.ok(html.includes('ov-oor-line') && html.includes('レビュー対象外 · src/b.rs（+2000 −0） ほか 1 件'));
  assert.ok(html.includes('data-ov-split="acme/widget#1"') && html.includes('call_split'));
  assert.ok(!html.includes('data-ov-ai') && !html.includes('AI 下読み</button>') && !html.includes('menu_book'));
  // Disabled while no read can be asked for, and only marked so while the request is out.
  assert.match(rowOf(big, { canRead: false }), /data-ov-split="acme\/widget#1" disabled title="hub is down"/);
  const asking = rowOf(big, { asking: true });
  assert.ok(asking.includes('aria-disabled="true"') && asking.includes('依頼中…'));
  // A PR that is not out of review keeps its AI 下読み, and has no such line.
  const plainRow = rowOf(rec(1, { files: [file('src/a.rs', 100)] }));
  assert.ok(plainRow.includes('data-ov-ai') && !plainRow.includes('data-ov-split') && !plainRow.includes('ov-oor-line'));
  // Waiting on its author, the row has neither the line nor a button.
  const waiting = rowOf(rec(1, { state: 'waiting-on-author', files: [file('src/a.rs', 5000)] }));
  assert.ok(!waiting.includes('data-ov-split') && !waiting.includes('ov-oor-line'));
  // While a read is at work there is no button, whichever mode.
  const reading = rowOf(rec(1, { state: 'ai-reading', ai: { status: 'running', mode: 'split' }, files: [file('src/a.rs', 5000)] }));
  assert.ok(reading.includes('分割案を作成中') && !reading.includes('data-ov-split') && !reading.includes('data-ov-ai'));
});

test('the buttons are drawn beside the row button, and the state line ends with an invisible copy of them', () => {
  const html = rowOf(rec(1, { files: [file('src/a.rs', 100)] }), {});
  const row = html.indexOf('</button>');
  // The real buttons follow the row button, in a layer of their own.
  const acts = html.indexOf('<span class="wk-acts ov-acts">');
  assert.ok(row > 0 && acts > row && html.indexOf('data-ov-ai=') > acts);
  assert.ok(!html.slice(0, row).includes('data-ov-ai'));
  // The ghost sits in the state line, hidden from assistive technology, with the label and no hook.
  const line = html.match(/<span class="wk-line ov-line[^"]*">[\s\S]*?<\/span><\/span>/)[0];
  assert.match(line, /<span class="ov-acts-ghost" aria-hidden="true"><span class="ov-act">[\s\S]*AI 下読み<\/span><\/span>$/);
  assert.ok(!line.includes('data-ov-') && !line.includes('</button>'));
  // The new mark is the second action, in both.
  const fresh = ctx.othersRowHtml(rec(1, { files: [file('src/a.rs', 100)] }), true, { canRead: true }, T0 + 60, [rec(1)]);
  assert.ok(fresh.indexOf('data-ov-ai=') < fresh.indexOf('data-ov-read=') && fresh.includes('<span class="wk-act"><span'));
  // The line of a PR out of review comes above the state line, which stays the last line.
  const big = rowOf(rec(1, { files: [file('src/b.rs', 2000)] }));
  assert.ok(big.indexOf('ov-oor-line') < big.indexOf('wk-line ov-line'));
  // Nothing to offer: no layer and no ghost.
  const none = rowOf(rec(1, { state: 'waiting-on-author', files: [file('src/a.rs', 100)] }));
  assert.ok(!none.includes('ov-acts') && !none.includes('ov-acts-ghost'));
});

test('a row with no state text keeps its state line only for the ghost', () => {
  const { othersStateLine: stateLine, othersAiView: aiView } = ctx;
  const row = (text, action) => {
    Object.assign(ctx, { othersStateLine: () => ({ cls: '', text }), othersAiView: () => ({ text: '', action }) });
    try { return rowOf(rec(1, { files: [file('src/a.rs', 100)] })); } finally { Object.assign(ctx, { othersStateLine: stateLine, othersAiView: aiView }); }
  };
  assert.match(row('', 'AI 下読み'), /ov-line ov-line-bare"><span class="ov-acts-ghost"/);
  assert.ok(!row('', null).includes('ov-line'));
});

test('what a button asks for follows whether the PR is out of review now, not the mode of the stored proposal', () => {
  const small = [file('src/a.rs', 100)];
  // Out of review now: the proposal is asked for again.
  const big = rowOf(rec(1, { files: [file('src/big.rs', 2000)], ai: { status: 'failed', mode: 'split', error: 'x' } }));
  assert.ok(big.includes('data-ov-split') && !big.includes('data-ov-ai'));
  // Small now (the author pushed): the usual button, in the usual words, though the status line keeps speaking of the proposal.
  const failed = rowOf(rec(1, { files: small, ai: { status: 'failed', mode: 'split', error: 'x' } }));
  assert.ok(failed.includes('data-ov-ai') && failed.includes('もう一度 AI 下読み') && failed.includes('分割案の作成に失敗しました') && !failed.includes('data-ov-split'));
  const stale = rowOf(rec(1, { state: 'ai-ready', files: small, ai: { status: 'ready', mode: 'split', stale: true, sha: 'abcdef0123' } }));
  assert.ok(stale.includes('data-ov-ai') && stale.includes('下読みし直す') && stale.includes('分割案あり') && !stale.includes('data-ov-split'));
  assert.deepStrictEqual(plain(othersAiView(rec(1, { state: 'ai-ready', files: small, ai: { status: 'ready', mode: 'split', stale: true, sha: 'abcdef0123' } }))),
    { kind: 'stale', text: '分割案あり（前の push 時点 abcdef0）', action: '下読みし直す' });
});

test('a current proposal of a PR that is not out of review now offers the usual read, and one of a PR that is keeps no button', () => {
  const ai = { status: 'ready', mode: 'split', stale: false, sha: 'abcdef0123' };
  const small = rowOf(rec(1, { state: 'ai-ready', files: [file('src/a.rs', 100)], ai }));
  assert.ok(small.includes('data-ov-ai') && small.includes('分割案あり') && !small.includes('data-ov-split'));
  assert.deepStrictEqual(plain(othersAiView(rec(1, { state: 'ai-ready', files: [file('src/a.rs', 100)], ai }))), { kind: 'done', text: '分割案あり', action: 'AI 下読み' });
  const big = rowOf(rec(1, { state: 'ai-ready', files: [file('src/big.rs', 2000)], ai }));
  assert.ok(!big.includes('data-ov-ai') && !big.includes('data-ov-split'));
  // Waiting on its author: nothing is offered.
  assert.strictEqual(othersAiView(rec(1, { state: 'waiting-on-author', files: [], ai })).action, null);
});

test('read-all marks only the new records, hidden or not, with the document\'s now', () => {
  const rows = [rec(1), rec(2, { events: { requested: stamp(T0), rerequested: null, aiReady: null, pushed: null } }), rec(3, { state: 'waiting-on-author' }), rec(4, { repo: 'Other/Repo', id: 'other/repo#4' })];
  const marks = { 'acme/widget#2': { read: T0 } };
  assert.deepStrictEqual(plain(othersReadAllPatches(rows, marks, T0 + 99)), [['acme/widget#1', { read: T0 + 99 }], ['other/repo#4', { read: T0 + 99 }]]);
  assert.deepStrictEqual(plain(othersReadAllPatches(rows, { 'acme/widget#1': { read: T0 }, 'acme/widget#2': { read: T0 }, 'other/repo#4': { read: T0 } }, T0 + 99)), []);
});

/* The list is drawn by my-work-others-view.js too: its function is cut out, with the row and the hub drawn as stubs. */
function listOf(doc, { repo = '', stage = '', marks = {}, hub = '', hideDone } = {}) {
  const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
  const cut = (src, re) => { const m = src.match(re); assert.ok(m); return m[0]; };
  Object.assign(ctx, { others: { doc, error: null }, prefs: { othersRepo: repo, othersStage: stage, othersHideDone: hideDone }, othersMarks: () => marks, othersNowEst: () => T0 + 60,
    othersSyncButtonHtml: () => '<sync>', othersHubHtml: () => hub, othersNotes: () => [], othersRowHtml: () => '<row>' });
  vm.runInContext(cut(read('my-work.js'), /^function workReadAllButtonHtml[\s\S]*?^}/m), ctx);
  vm.runInContext(cut(read('my-work-others-view.js'), /^function othersListHtml[\s\S]*?^}/m), ctx);
  return ctx.othersListHtml();
}
const listDoc = records => ({ now: T0 + 60, lastSync: { at: stamp(T0) }, records });

test('the 「AI 下読み」 list offers 新着をすべて既読にする only while a row is new, and not while it loads', () => {
  const withNew = listOf(listDoc([rec(1)]));
  assert.ok(withNew.includes('data-ov-read-all') && !withNew.includes('data-wk-read-all'));
  assert.ok(!listOf(listDoc([rec(1)]), { marks: { 'acme/widget#1': { read: T0 } } }).includes('data-ov-read-all'));
  assert.ok(!listOf(null).includes('data-ov-read-all'));
});

test('the read-all button is the 「タスク」 list\'s button', () => {
  const html = listOf(listDoc([rec(1)]));
  assert.ok(html.includes(ctx.workReadAllButtonHtml('data-ov-read-all')));
  for (const s of ['class="wk-act"', 'done_all', 'aria-label="新着をすべて既読にする"']) assert.ok(html.includes(s), s);
});

test('it shows when the new row is in a repository the filter hides', () => {
  const read = rec(1);
  const hidden = rec(2, { repo: 'Other/Repo', id: 'other/repo#2' });
  const html = listOf(listDoc([read, hidden]), { repo: 'acme/widget', marks: { 'acme/widget#1': { read: T0 } } });
  assert.ok(html.includes('data-ov-read-all'));
});

const ready = { status: 'ready', stale: false, sha: 'abcdef0123' };
const NOW = T0 + 60;
const stageOf = (extra, records) => { const r = rec(1, extra); return othersAiStage(r, records || [r], NOW); };

test('where an AI 下読み stands, for every kind it can be in', () => {
  const split = { ...ready, mode: 'split' };
  const big = [{ path: 'src/big.rs', additions: 2000, deletions: 0 }];
  assert.strictEqual(stageOf({ state: 'ai-ready', ai: ready }), 'done');
  assert.strictEqual(stageOf({ state: 'ai-ready', ai: { ...ready, stale: true } }), 'done');
  assert.strictEqual(stageOf({ state: 'ai-ready', ai: split }), 'done');
  assert.strictEqual(stageOf({ state: 'ai-ready', files: big, ai: split }), 'done');
  assert.strictEqual(stageOf({ state: 'ai-ready', files: big, ai: { ...split, stale: true } }), 'done');
  assert.strictEqual(stageOf({ ai: { status: 'queued', startedAt: stamp(T0) } }), 'reading');
  assert.strictEqual(stageOf({ state: 'ai-reading', ai: { status: 'running', launchedAt: stamp(T0) } }), 'reading');
  assert.strictEqual(stageOf({ state: 'ai-reading', ai: { status: 'running', launchedAt: stamp(NOW - 1801) } }), 'none');
  assert.strictEqual(stageOf({ ai: { status: 'failed', error: 'x' } }), 'none');
  assert.strictEqual(stageOf({ ai: { status: 'failed', readerClosedAt: stamp(T0) } }), 'none');
  assert.strictEqual(stageOf({}), 'none');
  assert.strictEqual(stageOf({ ai: { status: 'mystery' } }), 'none');
});

test('an AI 下読み filter that is not one of the stages is すべて', () => {
  assert.strictEqual(othersEffectiveStage('done'), 'done');
  for (const s of [undefined, null, 'bogus', '', 'DONE']) assert.strictEqual(othersEffectiveStage(s), '');
});

const stageRows = () => [rec(1, { state: 'ai-ready', ai: ready }), rec(2),
  rec(3, { repo: 'Zeta/App', id: 'zeta/app#3' }), rec(4, { repo: 'Zeta/App', id: 'zeta/app#4', state: 'ai-reading', ai: { status: 'running', launchedAt: stamp(T0) } })];

test('the AI 下読み filter ANDs with the repository, and drops the groups it empties', () => {
  const rows = stageRows();
  assert.deepStrictEqual(plain(othersGroups(rows, '', 'done', NOW).map(g => [g.key, g.rows.map(r => r.number)])), [['acme/widget', [1]]]);
  assert.deepStrictEqual(plain(othersGroups(rows, '', 'none', NOW).map(g => [g.key, g.rows.map(r => r.number)])), [['acme/widget', [2]], ['zeta/app', [3]]]);
  assert.deepStrictEqual(plain(othersGroups(rows, 'zeta/app', 'none', NOW).map(g => [g.key, g.rows.map(r => r.number)])), [['zeta/app', [3]]]);
  // The repository that applies is judged over all the records, so a stage it has no row in shows nothing, not another repository's rows.
  assert.deepStrictEqual(plain(othersGroups(rows, 'zeta/app', 'done', NOW)), []);
});

test('the AI 下読み chips count the repository that applies', () => {
  const rows = stageRows();
  const counts = repo => plain(othersStageChips(rows, repo, NOW).map(c => [c.key, c.count]));
  assert.deepStrictEqual(counts(''), [['', 4], ['done', 1], ['reading', 1], ['none', 2]]);
  assert.deepStrictEqual(counts('zeta/app'), [['', 2], ['done', 0], ['reading', 1], ['none', 1]]);
  assert.deepStrictEqual(counts('gone/repo'), counts(''));
  assert.deepStrictEqual(plain(othersStageChips(rows, '', NOW).map(c => c.label)), ['すべて', '下読み済み', '下読み中', '未']);
});

test('a stage with no row says so, keeps its chip pressed, and leaves the rest of the list', () => {
  const html = listOf(listDoc([rec(1), rec(2)]), { stage: 'reading' });
  assert.ok(html.includes('「下読み中」の PR はありません'));
  assert.ok(/data-ov-stage="reading" aria-pressed="true">下読み中 <span class="ov-chip-count">0<\/span>/.test(html));
  assert.ok(html.includes('<sync>') && html.includes('data-ov-read-all') && !html.includes('<row>'));
  const some = listOf(listDoc([rec(1), rec(2)]), { stage: 'none' });
  assert.ok(some.includes('<row>') && !some.includes('の PR はありません'));
  assert.ok(some.includes('aria-label="AI 下読みで絞り込む"') && some.includes('data-ov-stage="done"'));
  assert.ok(!listOf(listDoc([]), { stage: 'done' }).includes('data-ov-stage'));
});

test('read-all still shows when the only new row is hidden by the AI 下読み filter', () => {
  const read = rec(1, { state: 'ai-ready', ai: ready });
  const hidden = rec(2);
  const html = listOf(listDoc([read, hidden]), { stage: 'done', marks: { 'acme/widget#1': { read: T0 + 100 } } });
  assert.ok(html.includes('data-ov-read-all'));
});

test('a row marks where its AI 下読み stands', () => {
  assert.ok(rowOf(rec(1, { state: 'ai-ready', ai: ready })).includes('ov-stage ov-stage-done">下読み済み'));
  assert.ok(rowOf(rec(1)).includes('ov-stage ov-stage-none">未下読み'));
  assert.ok(rowOf(rec(1, { state: 'ai-reading', ai: { status: 'running', launchedAt: stamp(T0) } })).includes('ov-stage ov-stage-reading">下読み中'));
});

test('the 「レビュー」 list starts with the code-review hub, above the sync row', () => {
  const html = listOf(listDoc([rec(1)]), { hub: '<hub-state>' });
  assert.ok(html.startsWith('<hub-state>') && html.indexOf('<hub-state>') < html.indexOf('ov-sync-row'));
});

const finished = (n, reason, extra = {}) => rec(n, { state: 'done', doneReason: reason, ...extra });
const doneRows = () => [finished(10, 'merged'), finished(11, 'closed'), finished(12, 'withdrawn'), finished(13, 'approved')];
const rowCount = html => (html.match(/<row>/g) || []).length;

test('finished PRs are left out unless the saved choice is off', () => {
  for (const v of [undefined, true, null, 'x', 0]) assert.strictEqual(othersHidesDone(v), true);
  assert.strictEqual(othersHidesDone(false), false);
});

test('hiding finished PRs drops every done record and keeps the rest', () => {
  const rows = [...doneRows(), rec(1), rec(2, { state: 'waiting-on-author' }), rec(3, { state: 'ai-ready' }), rec(4, { state: 'something-new' })];
  assert.deepStrictEqual(plain(othersShown(rows, true).map(r => r.number)), [1, 2, 3, 4]);
  assert.strictEqual(othersShown(rows, false).length, 8);
  assert.deepStrictEqual(plain(othersShown(null, true)), []);
  assert.deepStrictEqual(plain(othersShown(null, false)), []);
});

test('a repository whose PRs are all finished has no group, no chip and is not an effective repository', () => {
  const rows = [rec(1), finished(2, 'merged', { repo: 'Gone/App', id: 'gone/app#2' })];
  const shown = othersShown(rows, true);
  assert.deepStrictEqual(plain(othersGroups(shown, '', '', NOW).map(g => g.key)), ['acme/widget']);
  assert.deepStrictEqual(plain(othersRepoChips(shown).map(c => c.key)), ['acme/widget']);
  assert.strictEqual(othersEffectiveRepo(shown, 'gone/app'), '');
  assert.strictEqual(othersEffectiveRepo(rows, 'gone/app'), 'gone/app');
});

test('the AI 下読み chips count a finished unread row only while it is shown', () => {
  const rows = [rec(1), finished(2, 'merged')];
  const none = shown => othersStageChips(shown, '', NOW).find(c => c.key === 'none').count;
  assert.strictEqual(none(othersShown(rows, true)), 1);
  assert.strictEqual(none(othersShown(rows, false)), 2);
});

test('the list has the toggle, pressed by default, and draws finished rows only when it is off', () => {
  const doc = listDoc([rec(1), finished(2, 'merged')]);
  const on = listOf(doc);
  assert.ok(/data-ov-done aria-pressed="true"[^>]*>終わったものを除く/.test(on));
  assert.strictEqual(rowCount(on), 1);
  const off = listOf(doc, { hideDone: false });
  assert.ok(/data-ov-done aria-pressed="false"[^>]*>終わったものを除く/.test(off));
  assert.strictEqual(rowCount(off), 2);
  assert.ok(!listOf(listDoc([])).includes('data-ov-done'));
});

test('a list of only finished PRs says how many are hidden and offers to show them', () => {
  const doc = listDoc([finished(2, 'merged')]);
  const html = listOf(doc);
  assert.ok(html.includes('終わっていない PR はありません。終わった PR 1 件を隠しています'));
  assert.ok(/class="ov-sync-btn lite" data-ov-done aria-pressed="false">[\s\S]*終わったものも表示/.test(html));
  assert.strictEqual(rowCount(html), 0);
  const off = listOf(doc, { hideDone: false });
  assert.strictEqual(rowCount(off), 1);
  assert.ok(!off.includes('PR はありません'));
});

test('a stage with only finished rows says so with the count hidden, and a stage with nothing keeps the old text', () => {
  const doc = listDoc([rec(1, { state: 'ai-ready', ai: ready }), finished(2, 'closed'), finished(3, 'approved')]);
  const html = listOf(doc, { stage: 'none' });
  assert.ok(html.includes('「未」の PR はありません。終わった PR 2 件を隠しています'));
  assert.ok(html.includes('終わったものも表示'));
  assert.ok(/data-ov-stage="none" aria-pressed="true">未 <span class="ov-chip-count">0<\/span>/.test(html));
  const empty = listOf(listDoc([rec(1)]), { stage: 'reading' });
  assert.ok(empty.includes('「下読み中」の PR はありません') && !empty.includes('隠しています') && !empty.includes('終わったものも表示'));
});

test('a saved repository whose PRs are all finished falls back to すべて, and the hidden count uses that repository', () => {
  const doc = listDoc([rec(1, { state: 'ai-ready', ai: ready }), finished(2, 'merged', { repo: 'Gone/App', id: 'gone/app#2' }), finished(3, 'merged')]);
  const html = listOf(doc, { repo: 'gone/app', stage: 'none' });
  // すべて repositories: the two finished rows are hidden, not the one of the saved repository only.
  assert.ok(html.includes('「未」の PR はありません。終わった PR 2 件を隠しています'));
  assert.ok(/data-ov-repo="" aria-pressed="true"/.test(html));
});

test('hiding finished PRs changes neither the rail\'s count nor 新着', () => {
  const rows = [rec(1), finished(2, 'merged')];
  assert.deepStrictEqual(othersWaiting(rows).map(r => r.number), [1]);
  const doc = listDoc([finished(2, 'merged', { events: { requested: stamp(T0), rerequested: null, aiReady: null, pushed: null } }), rec(3, { state: 'waiting-on-author' })]);
  assert.strictEqual(othersNewIds(rows, {}).length, 1);
  assert.ok(!listOf(doc).includes('data-ov-read-all') === (othersNewIds(doc.records, {}).length === 0));
});
