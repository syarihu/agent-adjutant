/* A PR waiting on your review as markup and as data (src/ui/pr-read.js), run with `node --test`. The page's scripts share
   one global scope, so they are loaded into one context the way the browser does; `secTitle` and `panelBtnsHtml` belong to the
   task panel, which is not loaded, so they are stood in for. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({
  secTitle: text => `<div class="tp-sec-title">${text}</div>`,
  panelBtnsHtml: jump => `<div class="tp-head-btns">${jump}<button data-tp-close></button></div>`,
});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const run = expr => vm.runInContext(expr, ctx);
const { prPaneOf, prTabsHtml, prDetailHtml, prBeforeRead, prSeedDrafts, prNewReaderDrafts, prDraftFromPoint, prDraftLocation, prCopyAllText,
  prDialogGroups, prNextKey, prDraftsHtml, prHeadHtml, prMiddleHtml, prPendable } = ctx;

const T0 = Date.UTC(2026, 9, 1) / 1000;
const A = 'a'.repeat(40);
const B = 'b'.repeat(40);
const rec = (extra = {}) => ({
  id: 'acme/widget#12', repo: 'acme/widget', number: 12, url: 'https://github.com/acme/widget/pull/12', title: 'Round the amounts',
  author: 'alice', base: 'main', head: 'feat', headSha: A, registered: true, state: 'ai-ready', additions: 30, deletions: 4, changedFiles: 3,
  files: [{ path: 'src/a.rs', additions: 20, deletions: 3, change: 'modified' }, { path: 'src/b.rs', additions: 8, deletions: 1, change: 'modified' },
    { path: 'docs/x.md', additions: 2, deletions: 0, change: 'added' }],
  ci: { pass: 3, fail: 0, pending: 1 }, reviewers: [{ login: 'bob', team: false, state: 'approved' }, { login: 'acme/core', team: true, state: 'requested' }],
  events: { pushed: null }, ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z', stale: false }, ...extra,
});
const read = (extra = {}) => ({
  sha: A, madeAt: '20261001T000000Z', three: { purpose: 'P', approach: 'Ap', impact: 'Im', essence: 'Es' },
  points: [{ level: 'attention', path: 'src/a.rs', line: 42, title: 'Rounding edge', why: 'W1' }, { level: 'info', path: 'docs/x.md', title: 'Doc', why: 'W2' }],
  order: [{ path: 'src/b.rs', category: 'logic', note: 'N', skippable: false }, { path: 'src/a.rs', category: 'test', note: '', skippable: true }],
  deleted: [{ path: 'old.rs', reason: 'moved', question: null }, { path: 'gone.rs', reason: null, question: 'why?' }],
  decisions: [{ decision: 'D', gains: 'G', costs: 'C' }], tests: { covered: ['T1'], notCovered: ['T2'] }, unwritten: ['Q1'],
  drafts: [{ path: 'src/a.rs', line: 42, side: 'right', body: 'B1' }], overall: { body: 'OB', suggestion: 'Request changes' },
  sinceReview: null, ...extra,
});
const hub = { canRead: true, why: '' };
const secKeys = html => [...html.matchAll(/data-pr-sec="([^"]*)"/g)].map(m => m[1]);

test('only 下読み opens before a read-through, and 下書き and 差分 need one (差分 also the annotated diff)', () => {
  assert.strictEqual(prPaneOf('detail', true), 'detail');
  assert.strictEqual(prPaneOf('drafts', false), 'detail');
  assert.strictEqual(prPaneOf('drafts', true), 'drafts');
  assert.strictEqual(prPaneOf('diff', true, false), 'detail');
  assert.strictEqual(prPaneOf('diff', true, true), 'diff');
  assert.strictEqual(prPaneOf('nonsense', true, true), 'detail');
});

test('the tabs: a dot on 下読み only when pushed, the others disabled before a read, a count on 下書き', () => {
  const quiet = prTabsHtml(rec(), read(), 'detail', 0);
  assert.ok(!quiet.includes('tp-dot'));
  assert.ok(prTabsHtml(rec({ events: { pushed: '20261001T000000Z' } }), read(), 'detail', 0).includes('tp-dot'));
  const before = prTabsHtml(rec({ ai: null }), null, 'detail', 0);
  assert.strictEqual((before.match(/ disabled /g) || []).length, 2);
  assert.ok(before.includes('AI 下読みのあとに使えます'));
  // With a read, 差分 stays disabled until there is a diff to show.
  const after = prTabsHtml(rec(), read(), 'drafts', 3, false);
  assert.strictEqual((after.match(/ disabled /g) || []).length, 1);
  assert.ok(after.includes('差分はまだ表示できません'));
  assert.ok(after.includes('>3</span>'));
  assert.ok(after.includes('data-pr-pane="drafts" aria-selected="true"'));
  assert.strictEqual((prTabsHtml(rec(), read(), 'detail', 0, true).match(/ disabled /g) || []).length, 0);
});

test('the 下読み tab is in the issue\'s order, with 前回のレビューから first after the facts for a PR that was pushed', () => {
  const html = prDetailHtml(rec(), read(), [rec()], hub, T0);
  assert.deepStrictEqual(secKeys(html), ['top:facts', 'top:three', 'points:list', 'order:list', 'more:decisions', 'more:tests', 'more:unwritten', 'footer']);
  const pushed = rec({ state: 'pushed', commitsSinceReview: 2 });
  const withSince = read({ sinceReview: { commit: B, comments: [{ commentId: 1, path: 'src/a.rs', line: 7, status: 'untouched', note: 'still' }], newChanges: [{ path: 'src/b.rs', note: 'new' }] } });
  const h2 = prDetailHtml(pushed, withSince, [pushed], hub, T0);
  assert.deepStrictEqual(secKeys(h2).slice(0, 3), ['top:facts', 'since:list', 'top:three']);
  assert.ok(h2.includes('前回のレビューから +2 コミット'));
  assert.ok(h2.includes('手つかず') && h2.includes('src/a.rs:7') && h2.includes('新しく増えた変更'));
  // Before a read there is the AI 下読み card, and the count line says what the read will check.
  const none = rec({ ai: null, state: 'pushed', commitsSinceReview: 2 });
  const h3 = prDetailHtml(none, null, [none], hub, T0);
  assert.deepStrictEqual(secKeys(h3), ['top:facts', 'since:list', 'top:ai']);
  assert.ok(h3.includes('data-pr-ai') && h3.includes('下読みするまでは GitHub の情報だけです'));
});

test('the footer offers 「下読みし直す」 only when the head has moved', () => {
  assert.ok(!prDetailHtml(rec(), read(), [rec()], hub, T0).includes('data-pr-reread'));
  const stale = rec({ headSha: B, ai: { status: 'ready', sha: A, stale: true } });
  const html = prDetailHtml(stale, read(), [stale], hub, T0);
  assert.ok(html.includes('data-pr-reread') && html.includes('head は bbbbbbb') && html.includes('下読みした時点: <code>aaaaaaa</code>'));
  const off = prDetailHtml(stale, read(), [stale], { canRead: false, why: 'stopped' }, T0);
  assert.match(off, /data-pr-reread disabled title="stopped"/);
});

test('everything the reader or the PR wrote comes out as text', () => {
  const bad = '<img src=x onerror=alert(1)>';
  const r = read({
    three: { purpose: bad, approach: bad, impact: bad, essence: bad },
    points: [{ level: 'check', path: bad, line: 1, title: bad, why: bad }], order: [{ path: bad, category: 'logic', note: bad, skippable: false }],
    deleted: [{ path: bad, reason: bad, question: null }], decisions: [{ decision: bad, gains: bad, costs: bad }], tests: { covered: [bad], notCovered: [bad] },
    unwritten: [bad], sinceReview: { commit: B, comments: [{ commentId: 1, path: bad, status: bad, note: bad }], newChanges: [{ path: bad, note: bad }] },
  });
  const pr = rec({ title: bad, author: bad, repo: bad, base: bad, head: bad, state: 'pushed', reviewers: [{ login: bad, team: false, state: bad }] });
  const html = prDetailHtml(pr, r, [pr], hub, T0) + prHeadHtml(pr) + prDialogGroups(pr, r, [pr], hub, T0).map(g => g.cards.map(c => c.html).join('')).join('')
    + prDraftsHtml({ drafts: [{ key: 'k', source: 'person', path: bad, body: bad }], overall: { body: bad } }, r, { notice: 0 });
  assert.ok(!html.includes('<img'), html.match(/.{20}<img.{20}/)?.[0]);
  assert.ok(html.includes('&lt;img'));
});

test('a link that is not http(s) is no link, and the PR\'s own opens in a new tab without the opener', () => {
  const head = prHeadHtml(rec());
  assert.match(head, /href="https:\/\/github\.com\/acme\/widget\/pull\/12" target="_blank" rel="noopener noreferrer"/);
  assert.ok(head.includes('data-pr-expand') && head.includes('aria-haspopup="dialog"') && head.includes('GitHub で開く'));
  assert.ok(!prHeadHtml(rec({ url: 'javascript:alert(1)' })).includes('javascript:'));
  assert.ok(prHeadHtml(rec({ registered: false })).includes('未登録'));
});

test('before a read: the button is disabled with the hub\'s reason, and a queued read says its place', () => {
  const r = rec({ ai: null, state: 'requested' });
  const off = prBeforeRead(r, [r], { canRead: false, why: 'hub が止まっています' }, T0);
  assert.match(off.html, /data-pr-ai disabled title="hub が止まっています"/);
  assert.ok(prBeforeRead(r, [r], hub, T0, true).html.includes('依頼中…'));
  const q = rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' }, state: 'ai-reading' });
  const other = rec({ id: 'acme/widget#9', ai: { status: 'queued', startedAt: '20260930T000000Z' }, state: 'ai-reading' });
  assert.ok(prBeforeRead(q, [q, other], hub, T0).html.includes('順番待ち（2 番目）'));
  assert.ok(!prBeforeRead(q, [q, other], hub, T0).html.includes('data-pr-ai'));
});

test('the middle before a terminal: a note and one button, and 再接続 when the worker has ended', () => {
  const none = rec({ ai: null, state: 'requested' });
  const html = prMiddleHtml(none, [none], hub, T0);
  assert.ok(html.includes('まだ AI 下読みしていません') && html.includes('data-pr-ai'));
  const ended = prMiddleHtml(rec(), [rec()], hub, T0, { ended: 4404 });
  assert.ok(ended.includes('AI 下読みの worker は終わっています') && ended.includes('data-pr-reconnect'));
  const noWindow = prMiddleHtml(rec({ ai: { status: 'running', sha: null, launchedAt: '20261001T000000Z' } }), [], hub, T0);
  assert.ok(noWindow.includes('tmux のウィンドウが記録されていません'));
});

test('the reader\'s drafts seed the document with keys r1.., and what is already there is not offered again', () => {
  const r = read({ drafts: [{ path: 'a', line: 1, side: 'right', body: 'x' }, { path: 'b', body: 'y', line: 2, side: 'left' }] });
  const doc = plain(prSeedDrafts(r));
  assert.deepStrictEqual(doc.drafts.map(d => [d.key, d.source]), [['r1', 'reader'], ['r2', 'reader']]);
  assert.strictEqual(doc.fromSha, A);
  assert.strictEqual(doc.overall.body, 'OB');
  assert.deepStrictEqual(plain(prSeedDrafts(null)), { version: 1, fromSha: null, drafts: [], overall: { body: '' } });
  // After a re-read: one draft is the same, one is new, one changed. A key is never reused.
  const again = read({ sha: B, drafts: [{ path: 'a', line: 1, side: 'right', body: 'x' }, { path: 'c', line: 3, body: 'z', side: 'right' }] });
  const fresh = plain(prNewReaderDrafts({ drafts: [...doc.drafts, { key: `r${B.slice(0, 7)}-2`, source: 'person', path: 'q', body: '' }] }, again));
  assert.deepStrictEqual(fresh.map(d => [d.path, d.source]), [['c', 'reader']]);
  assert.strictEqual(fresh[0].key, `r${B.slice(0, 7)}-2-2`);
});

test('a point becomes the person\'s own draft: a file draft when it has no line', () => {
  const doc = { drafts: [{ key: 'p1' }, { key: 'p3' }] };
  assert.strictEqual(prNextKey(doc, 'p'), 'p2');
  assert.deepStrictEqual(plain(prDraftFromPoint({ level: 'check', path: 'src/a.rs', line: 5, title: 'T', why: 'W' }, doc)),
    { key: 'p2', source: 'person', path: 'src/a.rs', line: 5, side: 'right', body: 'T: W' });
  const file = plain(prDraftFromPoint({ level: 'info', path: 'docs/x.md', title: 'T', why: 'W' }, { drafts: [] }));
  assert.ok(!('line' in file));
  assert.strictEqual(file.key, 'p1');
});

test('where a draft is: a line, a range, a file, or the whole PR', () => {
  assert.strictEqual(prDraftLocation({ path: 'a.rs', line: 4 }), 'a.rs:4');
  assert.strictEqual(prDraftLocation({ path: 'a.rs', line: 4, startLine: 2 }), 'a.rs:2-4');
  assert.strictEqual(prDraftLocation({ path: 'a.rs' }), 'a.rs');
  assert.strictEqual(prDraftLocation({ body: 'x' }), 'PR 全体');
});

test('すべてコピー: each draft with its place, blank ones skipped, then the overall comment', () => {
  const doc = { drafts: [{ path: 'a.rs', line: 4, body: 'one' }, { path: 'b.rs', body: '  ' }, { body: 'whole' }], overall: { body: 'all' } };
  assert.strictEqual(prCopyAllText(doc), 'a.rs:4\none\n\nPR 全体\nwhole\n\n全体のコメント:\nall');
  assert.strictEqual(prCopyAllText({ drafts: [], overall: { body: '' } }), '');
});

test('the drafts: a box each with its place as its name, the suggestion as text, the notice and the save line', () => {
  const doc = { drafts: [{ key: 'r1', source: 'reader', path: 'a.rs', line: 4, body: 'x & y' }], overall: { body: 'ob' } };
  const html = prDraftsHtml(doc, read(), { place: 'dock', notice: 2, save: { status: 'error', error: 'boom' } });
  assert.ok(html.includes('data-pr-body="r1"') && html.includes('aria-label="a.rs:4 への下書き"') && html.includes('x &amp; y'));
  assert.ok(html.includes('AI の判定の目安: <b>Request changes</b>（文字だけ。判定を選んで送るのは GitHub です）'));
  assert.ok(html.includes('data-pr-take-new') && html.includes('下読みし直した下書きが 2 件あります'));
  assert.ok(html.includes('data-pr-pending-slot'));
  assert.ok(html.includes('保存できませんでした: boom 入力はこのページに残っています'));
  assert.ok(prDraftsHtml({ drafts: [], overall: null }, read(), {}).includes('下書きはまだありません'));
  assert.ok(!prDraftsHtml(doc, null, { notice: 0 }).includes('data-pr-take-new'));
});

test('what can be put into the pending review: a draft with a file and a body that is not in yet, and an overall comment that is new', () => {
  const doc = { drafts: [
    { key: 'a', path: 'a.rs', line: 1, body: 'one' }, { key: 'b', path: 'b.rs', body: 'file' },
    { key: 'c', path: 'c.rs', line: 2, body: '  ' }, { key: 'd', body: 'whole PR' },
    { key: 'e', path: 'e.rs', line: 3, body: 'sent', sentAt: '20261011T000000Z' }], overall: { body: 'all' } };
  assert.deepStrictEqual(plain(prPendable(doc)), { drafts: 2, overall: true });
  assert.deepStrictEqual(plain(prPendable({ ...doc, pending: { overall: 'all' } })), { drafts: 2, overall: false });
  assert.deepStrictEqual(plain(prPendable({ drafts: [], overall: { body: ' ' } })), { drafts: 0, overall: false });
  // What the server stored is normalized: the same text with a trailing newline or CRLF is the same comment.
  assert.deepStrictEqual(plain(prPendable({ drafts: [], overall: { body: 'LGTM\n' }, pending: { overall: 'LGTM' } })), { drafts: 0, overall: false });
  assert.deepStrictEqual(plain(prPendable({ drafts: [], overall: { body: 'a\r\nb  \n' }, pending: { overall: 'a\nb' } })), { drafts: 0, overall: false });
  assert.deepStrictEqual(plain(prPendable({ drafts: [], overall: { body: '\r\n \n' } })), { drafts: 0, overall: false });
  assert.deepStrictEqual(plain(prPendable(null)), { drafts: 0, overall: false });
});

test('the pending review button counts what is not in yet, is disabled with nothing to send, and says who sees it', () => {
  const doc = { drafts: [{ key: 'a', path: 'a.rs', line: 1, body: 'one' }, { key: 'b', path: 'b.rs', line: 2, body: 'two' }], overall: { body: '' } };
  const pend = { prUrl: 'https://github.com/acme/widget/pull/12' };
  const html = prDraftsHtml(doc, read(), { pend });
  assert.ok(/<button type="button" class="btn-m3-filled" data-pr-pend>2 件を pending review に積む<\/button>/.test(html), html);
  assert.ok(html.includes('pending review はあなたにしか見えません。判定を選んで送信するのは GitHub です。'));
  assert.ok(html.includes('href="https://github.com/acme/widget/pull/12/files" target="_blank" rel="noopener noreferrer">GitHub で仕上げる</a>'));
  // Once a review is noted, the link is to it.
  const noted = prDraftsHtml({ ...doc, pending: { url: 'https://github.com/acme/widget/pull/12#pullrequestreview-9' } }, read(), { pend });
  assert.ok(noted.includes('href="https://github.com/acme/widget/pull/12#pullrequestreview-9" target="_blank"') && !noted.includes('/files'));
  assert.ok(prDraftsHtml({ ...doc, pending: { url: 'javascript:alert(1)' } }, read(), { pend }).includes('pull/12/files'));
  assert.ok(prDraftsHtml({ drafts: [], overall: { body: 'only this' } }, read(), { pend }).includes('全体のコメントを pending review に積む'));
  const none = prDraftsHtml({ drafts: [], overall: { body: '' } }, read(), { pend });
  assert.ok(/data-pr-pend disabled title="[^"]+"/.test(none), none);
  assert.ok(none.includes('<div class="pr-pend-note">積めるものがありません。</div>') && !html.includes('積めるものがありません'));
  // No link for an address that is not http(s); the button is busy while it sends.
  assert.ok(!prDraftsHtml(doc, read(), { pend: { prUrl: 'javascript:alert(1)' } }).includes('GitHub で仕上げる'));
  const busy = prDraftsHtml(doc, read(), { pend: { ...pend, sending: true } });
  // Everything that could be typed into or changed is locked while it sends: the redraw after it would drop it.
  assert.ok(/class="pr-overall"[^>]* readonly>/.test(busy) && /data-pr-discard="a" disabled/.test(busy));
  assert.ok(!/class="pr-overall"[^>]* readonly>/.test(html) && !/data-pr-discard="a" disabled/.test(html));
  assert.ok(busy.includes('data-pr-pend disabled aria-busy="true"') && busy.includes('積んでいます…') && busy.includes(' readonly>'));
});

test('the result line says the overall comment went in too, and why one that was refused did not (it has no box)', () => {
  const doc = { drafts: [], overall: { body: 'o' } };
  const none = prDraftsHtml(doc, read(), { pend: { result: { sent: 0, failed: [], skipped: [], overall: 'set' } } });
  assert.ok(none.includes('0 件を積みました／全体のコメントも積みました'));
  const refused = prDraftsHtml(doc, read(), { pend: { result: { sent: 1, failed: [{ key: 'overall', why: '<i>no</i>' }], skipped: [], overall: 'none' } } });
  assert.ok(refused.includes('全体のコメントは積めませんでした: &lt;i&gt;no&lt;/i&gt;') && !refused.includes('全体のコメントも積みました'));
});

test('a draft that is in the pending review is read-only with a chip and keeps its copy button; one that was refused says why, as text', () => {
  const doc = { drafts: [{ key: 's1', source: 'person', path: 'a.rs', line: 1, body: 'in', sentAt: '20261011T000000Z' },
    { key: 'f1', source: 'person', path: 'b.rs', line: 2, body: 'out' }], overall: { body: '' } };
  const result = { sent: 1, failed: [{ key: 'f1', why: '<b>Line</b> must be part of the diff' }], skipped: [{ key: 'x', why: 'w' }] };
  const html = prDraftsHtml(doc, read(), { pend: { result, prUrl: rec().url } });
  const [sent, failed] = html.split('data-pr-draft="').slice(1);
  assert.ok(sent.includes('積みました</span>') && sent.includes('data-pr-copy="s1"') && /readonly>/.test(sent) && !sent.includes('積めませんでした'));
  assert.ok(!failed.includes('readonly') && !failed.includes('pr-chip-sent'));
  assert.ok(failed.includes('積めませんでした: &lt;b&gt;Line&lt;/b&gt; must be part of the diff') && !failed.includes('<b>Line'));
  assert.ok(html.includes('role="status">1 件を積みました／1 件は積めませんでした／1 件は対象外でした</div>'));
});

test('the dialog\'s groups: since, top, points, order, more, diff; before a read only the first two', () => {
  const pushed = rec({ state: 'pushed', commitsSinceReview: 3 });
  const r = read({ sinceReview: { commit: B, comments: [], newChanges: [] } });
  const groups = prDialogGroups(pushed, r, [pushed], hub, T0);
  assert.deepStrictEqual(plain(groups.map(g => g.key)), ['since', 'top', 'points', 'order', 'more', 'diff']);
  assert.strictEqual(groups[0].badge, '+3');
  assert.deepStrictEqual(plain(groups[1].cards.map(c => c.key)), ['top:facts', 'top:three', 'footer']);
  // The files in reading order, then the rest of the record's.
  assert.deepStrictEqual(plain(groups[5].cards.map(c => c.key)), ['diff:src/b.rs', 'diff:src/a.rs', 'diff:docs/x.md']);
  assert.ok(groups[5].cards[0].html.includes('src/b.rs（+8 −1）') && groups[5].cards[0].html.includes('GitHub で開いて読んでください'));
  // The keys are the same on a second call, which is what marks a change.
  assert.deepStrictEqual(plain(prDialogGroups(pushed, r, [pushed], hub, T0).map(g => [g.key, g.cards.map(c => c.key)])), plain(groups.map(g => [g.key, g.cards.map(c => c.key)])));
  const none = rec({ ai: null, state: 'pushed', commitsSinceReview: 3 });
  assert.deepStrictEqual(plain(prDialogGroups(none, null, [none], hub, T0).map(g => g.key)), ['since', 'top']);
  assert.deepStrictEqual(plain(prDialogGroups(rec({ ai: null, state: 'requested' }), null, [], hub, T0).map(g => g.key)), ['top']);
  assert.deepStrictEqual(plain(prDialogGroups(rec({ ai: null, state: 'requested' }), null, [], hub, T0)[0].cards.map(c => c.key)), ['top:facts', 'top:ai']);
});

test('with the annotated diff on: 読む順 paths are buttons, and the diff group takes the hook\'s head, paths and labels', () => {
  const r = read();
  const off = prDetailHtml(rec(), r, [rec()], hub, T0, {});
  assert.ok(!off.includes('data-pr-goto-file'));
  const on = prDetailHtml(rec(), r, [rec()], hub, T0, { diff: true });
  assert.ok(on.includes('<button type="button" class="pr-loc-btn" data-pr-goto-file="0"') && on.includes('data-pr-goto-file="1"'));
  assert.ok(on.includes('data-pr-goto-deleted="0"') && on.includes('data-pr-goto-deleted="1"'));
  // The index into the reader's lists, which is what the click reads the path from.
  assert.ok(on.indexOf('data-pr-goto-file="0"') < on.indexOf('src/b.rs') && on.indexOf('src/b.rs') < on.indexOf('data-pr-goto-file="1"'));
  const bad = prDetailHtml(rec(), read({ order: [{ path: '"><script>x</script>', category: 'logic', note: '', skippable: false }] }), [rec()], hub, T0, { diff: true });
  assert.ok(!bad.includes('<script>'));
  // The group: the head card first, the paths given (not the record's), their labels and titles.
  const groups = prDialogGroups(rec(), r, [rec()], hub, T0, {
    diff: true, diffHeadHtml: '<div>HEAD</div>', diffPaths: ['z/one.rs', 'y/two.rs'], diffFileHtml: p => `<i>${p}</i>`,
    diffLabel: p => ({ label: p.split('/').pop(), title: p }),
  });
  const diff = groups.find(g => g.key === 'diff');
  assert.deepStrictEqual(plain(diff.cards.map(c => [c.key, c.label, c.title, c.html])), [
    ['diff-bar', '差分の概要', '', '<div>HEAD</div>'], ['diff:z/one.rs', 'one.rs', 'z/one.rs', '<i>z/one.rs</i>'], ['diff:y/two.rs', 'two.rs', 'y/two.rs', '<i>y/two.rs</i>']]);
  assert.strictEqual(diff.badge, '2');
  // No paths yet (the diff is loading): the head card alone is the group.
  const loading = prDialogGroups(rec(), r, [rec()], hub, T0, { diff: true, diffHeadHtml: '<div>L</div>', diffPaths: [], diffFileHtml: () => '' }).find(g => g.key === 'diff');
  assert.deepStrictEqual(plain(loading.cards.map(c => c.key)), ['diff-bar']);
});

test('when to read the read-through again: its head or its finish time changed', () => {
  assert.strictEqual(run(`prReadKey(${JSON.stringify(rec())})`), `${A}|20261001T000000Z`);
  assert.strictEqual(run('prReadKey({})'), '|');
  assert.strictEqual(run(`prHasRead(${JSON.stringify(rec({ ai: null }))}, undefined)`), false);
  assert.strictEqual(run(`prHasRead(${JSON.stringify(rec())}, undefined)`), true);
  assert.strictEqual(run(`prHasRead(${JSON.stringify(rec())}, null)`), false);
});

test('a draft that starts with a newline keeps it: the markup adds the line feed the parser drops', () => {
  const html = prDraftsHtml({ drafts: [{ key: 'k', source: 'person', body: '\nafter' }], overall: { body: '\nall' } }, null, {});
  assert.ok(html.includes('>\n\nafter</textarea>'));
  assert.ok(html.includes('>\n\nall</textarea>'));
});

test('the dot is for a PR that waits on you and was pushed, not for one that is done', () => {
  assert.ok(prTabsHtml(rec({ state: 'pushed', events: { pushed: '20261001T000000Z' } }), read(), 'detail', 0).includes('tp-dot'));
  assert.ok(!prTabsHtml(rec({ state: 'done', events: { pushed: '20261001T000000Z' } }), read(), 'detail', 0).includes('tp-dot'));
});
