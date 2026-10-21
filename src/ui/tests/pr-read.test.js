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
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const run = expr => vm.runInContext(expr, ctx);
const { prPaneOf, prTabsHtml, prDetailHtml, prBeforeRead, prSeedDrafts, prNewReaderDrafts, prDraftFromPoint, prDraftLocation, prCopyAllText,
  prDialogGroups, prNextKey, prDraftsHtml, prReviewHeadHtml, prCiWord, prNoReadHtml, prMiddleHtml, prPendable, prHandable, prHandHtml,
  prOorActions, prOutOfReview, prSplitProposal, prNewReaderOverall, prFromElsewhere } = ctx;

const prCloseReaderBtnHtml = run('prCloseReaderBtnHtml');

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
  assert.ok(after.includes('data-mid-tab="drafts" aria-selected="true"'));
  // The tabs of the middle, as a task's are: the same class, named by `data-mid-tab`, for the read-through's place.
  assert.ok(after.includes('class="wk-mid-tab active"') && after.includes('aria-controls="task-read"') && after.includes('id="wk-mid-tab-pr-detail"'));
  assert.ok(!after.includes('data-pr-pane'));
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
  const html = prDetailHtml(pr, r, [pr], hub, T0) + prReviewHeadHtml(pr) + prDialogGroups(pr, r, [pr], hub, T0).map(g => g.cards.map(c => c.html).join('')).join('')
    + prDraftsHtml({ drafts: [{ key: 'k', source: 'person', path: bad, body: bad }], overall: { body: bad } }, r, { notice: 0 });
  assert.ok(!html.includes('<img'), html.match(/.{20}<img.{20}/)?.[0]);
  assert.ok(html.includes('&lt;img'));
});

test('a link that is not http(s) is no link, and the PR\'s own opens in a new tab without the opener', () => {
  const head = prReviewHeadHtml(rec());
  assert.match(head, /href="https:\/\/github\.com\/acme\/widget\/pull\/12" target="_blank" rel="noopener noreferrer"/);
  assert.ok(head.includes('data-pr-expand') && head.includes('aria-haspopup="dialog"') && head.includes('GitHub で開く') && head.includes('open_in_new'));
  const noLink = prReviewHeadHtml(rec({ url: 'javascript:alert(1)' }));
  assert.ok(!noLink.includes('javascript:') && !noLink.includes('GitHub で開く') && noLink.includes('data-pr-expand'));
});

test('the CI of a record is one word: a failure first, then a run under way, then a pass, else none', () => {
  assert.strictEqual(prCiWord({ pass: 3, fail: 2, pending: 1 }), '失敗 2');
  assert.strictEqual(prCiWord({ pass: 3, fail: 0, pending: 1 }), '実行中');
  assert.strictEqual(prCiWord({ pass: 3, fail: 0, pending: 0 }), '通過');
  assert.strictEqual(prCiWord({ pass: 0, fail: 0, pending: 0 }), 'なし');
  assert.strictEqual(prCiWord({}), 'なし');
  assert.strictEqual(prCiWord(null), 'なし');
  assert.strictEqual(prCiWord(undefined), 'なし');
});

test('the middle\'s heading: number and title, then repository, author, size, files and CI, and the two buttons at the right', () => {
  const head = prReviewHeadHtml(rec());
  assert.ok(head.includes('<div class="wk-term-title">#12 Round the amounts</div>'));
  assert.ok(head.includes('<div class="wk-term-sub">acme/widget · @alice · +30 −4 · 3 files · CI 実行中</div>'));
  const acts = head.slice(head.indexOf('wk-head-acts'));
  assert.ok(acts.indexOf('GitHub で開く') < acts.indexOf('data-pr-expand'));
  assert.ok(prReviewHeadHtml(rec({ author: null })).includes('（削除されたアカウント）'));
  assert.ok(prReviewHeadHtml(rec({ ci: { pass: 0, fail: 1, pending: 0 } })).includes('CI 失敗 1'));
  assert.ok(prReviewHeadHtml(rec({ title: '<b>x</b>' })).includes('#12 &lt;b&gt;x&lt;/b&gt;'));
});

test('before the first read the middle says it is under way, with the queue place, or that none was made, with the button', () => {
  const queued = rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' }, state: 'ai-reading' });
  const q = prNoReadHtml(queued, ctx.othersAiView(queued, [queued], T0), hub, false);
  assert.ok(q.includes('progress_activity') && q.includes('下読み中です') && q.includes('読み終わると、ここに要点と見るポイントが出ます。') && q.includes('順番待ち（1 番目）'));
  assert.ok(!q.includes('data-pr-ai'));
  const running = rec({ ai: { status: 'running', launchedAt: '20261001T000000Z' }, state: 'ai-reading' });
  const r = prNoReadHtml(running, ctx.othersAiView(running, [running], T0), hub, false);
  assert.ok(r.includes('下読み中です') && !r.includes('順番待ち'));
  const none = rec({ ai: null, state: 'requested' });
  const n = prNoReadHtml(none, ctx.othersAiView(none, [none], T0), hub, false);
  assert.ok(n.includes('auto_stories') && n.includes('まだ下読みしていません') && n.includes('コードレビュー hub の読み手が、要点と見るポイントをまとめます。'));
  assert.ok(n.includes('data-pr-ai') && n.includes('下読みする') && !n.includes('disabled'));
  // The hub's reason when no read can be asked for, and 依頼中… while a request is out.
  assert.match(prNoReadHtml(none, ctx.othersAiView(none, [none], T0), { canRead: false, why: 'hub が止まっています' }, false), /data-pr-ai disabled title="hub が止まっています"/);
  assert.ok(prNoReadHtml(none, ctx.othersAiView(none, [none], T0), hub, true).includes('依頼中…'));
  // The last read failed: why, and the button to try again.
  const failed = rec({ ai: { status: 'failed', error: 'boom' }, state: 'requested' });
  const f = prNoReadHtml(failed, ctx.othersAiView(failed, [failed], T0), hub, false);
  assert.ok(f.includes('まだ下読みしていません') && f.includes('boom') && f.includes('data-pr-ai'));
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
  const ended = prMiddleHtml(open({ status: 'ready' }), [open({ status: 'ready' })], hub, T0, { ended: 4404 });
  assert.ok(ended.includes('AI 下読みの worker は終わっています') && ended.includes('data-pr-reconnect'));
  // An attach that failed is not an end: it says why, and still offers 再接続.
  const failed = prMiddleHtml(open({ status: 'ready' }), [open({ status: 'ready' })], hub, T0, { failed: 'xterm.js を読み込めませんでした' });
  assert.ok(failed.includes('AI 下読みの worker に接続できませんでした') && failed.includes('xterm.js を読み込めませんでした') && failed.includes('data-pr-reconnect'));
  assert.ok(!failed.includes('終わっています'));
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

/* ── a PR whose change is too large to read ── */

const BIG = [{ path: 'src/big.rs', additions: 1400, deletions: 300, change: 'modified' }, { path: 'src/ok.rs', additions: 5, deletions: 1, change: 'modified' }];
const big = (extra = {}) => rec({ state: 'requested', ai: null, files: BIG, ...extra });
const splitRead = (extra = {}) => read({ mode: 'split', points: [], order: [], deleted: [], decisions: [], tests: { covered: [], notCovered: [] }, unwritten: [], drafts: [], overall: { body: 'PR1: split\nPR2: <b>rest</b>' }, ...extra });

test('a PR out of review has its own card right after the facts, and no AI 下読み card of the usual kind', () => {
  const r = big();
  const html = prDetailHtml(r, null, [r], hub, T0);
  assert.deepStrictEqual(secKeys(html), ['top:facts', 'top:oor']);
  assert.ok(html.includes('1 ファイルの変更が 1,500 行を超えているため'));
  assert.ok(html.includes('<code class="pr-loc">src/big.rs</code> +1400 −300（計 1700 行）') && !html.includes('src/ok.rs</code> +5'));
  assert.ok(html.includes('data-pr-split') && html.includes('分割を提案する') && html.includes('data-pr-ai') && html.includes('それでも AI 下読み'));
  assert.ok(html.includes('差分を全部読むので時間がかかります'));
  // A PR that is not out of review keeps its card, and one that waits for nothing of the person has no such card.
  assert.deepStrictEqual(secKeys(prDetailHtml(rec({ ai: null, state: 'requested' }), null, [], hub, T0)), ['top:facts', 'top:ai']);
  assert.strictEqual(prOutOfReview(big({ state: 'waiting-on-author' }), null, { kind: 'none' }, hub), null);
});

test('the buttons of a PR out of review: by what is stored, and none while a read is under way', () => {
  const view = r => plain(ctx.othersAiView(r, [r], T0));
  const labels = (r, rd) => plain(prOorActions(r, rd, view(r)));
  assert.deepStrictEqual(labels(big(), null), { split: '分割を提案する', full: 'それでも AI 下読み' });
  // A full read-through is stored: only the proposal is offered; once stale, the full read is offered again.
  assert.deepStrictEqual(labels(big({ ai: { status: 'ready', sha: A, stale: false }, state: 'ai-ready' }), read()), { split: '分割を提案する', full: null });
  assert.deepStrictEqual(labels(big({ ai: { status: 'ready', sha: A, stale: true }, state: 'ai-ready' }), read()), { split: '分割を提案する', full: 'それでも AI 下読み' });
  // A proposal is stored: only the full read is offered; stale, the proposal is offered to be made again.
  const proposal = { status: 'ready', sha: A, mode: 'split', stale: false };
  assert.deepStrictEqual(labels(big({ ai: proposal, state: 'ai-ready' }), splitRead()), { split: null, full: 'それでも AI 下読み' });
  assert.deepStrictEqual(labels(big({ ai: { ...proposal, stale: true }, state: 'ai-ready' }), splitRead()), { split: '分割を提案し直す', full: 'それでも AI 下読み' });
  // Under way in either mode: nothing; hung: both; failed: both.
  for (const mode of [undefined, 'split']) {
    assert.deepStrictEqual(labels(big({ ai: { status: 'running', mode, launchedAt: '20261001T000000Z' }, state: 'ai-reading' }), null), { split: null, full: null });
    assert.deepStrictEqual(labels(big({ ai: { status: 'queued', mode }, state: 'ai-reading' }), null), { split: null, full: null });
  }
  const hung = big({ ai: { status: 'running', launchedAt: '20260101T000000Z' }, state: 'ai-reading' });
  assert.deepStrictEqual(plain(prOorActions(hung, null, plain(ctx.othersAiView(hung, [hung], T0)))), { split: '分割を提案する', full: 'それでも AI 下読み' });
  // While the stored one is being fetched, nothing is offered.
  assert.deepStrictEqual(labels(big({ ai: proposal, state: 'ai-ready' }), undefined), { split: null, full: null });
});

test('a request in flight greys the buttons of a PR out of review, and the hub\'s reason is their title', () => {
  const r = big();
  assert.match(prOutOfReview(r, null, { kind: 'none' }, hub, true).html, /data-pr-split aria-disabled="true"[\s\S]*依頼中…/);
  assert.match(prOutOfReview(r, null, { kind: 'none' }, { canRead: false, why: 'stopped' }, false).html, /data-pr-split disabled title="stopped"/);
});

test('the middle of a PR out of review: the files, the two buttons, and what the worker reads; a read at work falls through', () => {
  const r = big();
  const html = prMiddleHtml(r, [r], hub, T0);
  assert.ok(html.includes('レビュー対象外') && html.includes('src/big.rs') && html.includes('data-pr-split') && html.includes('data-pr-ai'));
  assert.ok(html.includes('worker はファイルの構成と hunk の見出しだけを読んで、この PR の分割案を下書きにします。'));
  const failed = big({ ai: { status: 'failed', error: 'x', mode: 'split' }, state: 'requested' });
  assert.ok(prMiddleHtml(failed, [failed], hub, T0).includes('分割案の作成に失敗しました: x'));
  // A reader at work, in either mode, is the usual middle, with none of these buttons.
  for (const mode of [undefined, 'split']) {
    const at = big({ ai: { status: 'running', mode, launchedAt: '20261001T000000Z' }, state: 'ai-reading' });
    const h = prMiddleHtml(at, [at], hub, T0);
    assert.ok(!h.includes('data-pr-split') && !h.includes('data-pr-ai'), h);
    assert.ok(h.includes(mode ? '分割案を作成中' : 'AI 下読み中'));
  }
  // The proposal is in: its button is the usual one, and says it asks for a proposal.
  const done = big({ ai: { status: 'ready', sha: A, mode: 'split', stale: true }, state: 'ai-ready' });
  const stale = prMiddleHtml(done, [done], hub, T0, { read: splitRead() });
  // A stale proposal of a live PR out of review: the same block and buttons as the 下読み tab.
  assert.ok(stale.includes('分割案あり') && stale.includes('src/big.rs') && stale.includes('data-pr-split') && stale.includes('分割を提案し直す'));
  // A hung reader, and a finished read whose window cannot be opened, offer the ways on too.
  const hungOne = big({ ai: { status: 'running', launchedAt: '20260101T000000Z' }, state: 'ai-reading' });
  assert.ok(prMiddleHtml(hungOne, [hungOne], hub, T0, { read: null }).includes('data-pr-split'));
  const fullDone = big({ ai: { status: 'ready', sha: A, stale: false }, state: 'ai-ready' });
  const noWindow = prMiddleHtml(fullDone, [fullDone], hub, T0, { read: read() });
  assert.ok(noWindow.includes('data-pr-split') && !noWindow.includes('data-pr-ai'));
});

test('a split proposal is a card of its own, as text, and its read has no diff', () => {
  const r = big({ ai: { status: 'ready', sha: A, mode: 'split', stale: false }, state: 'ai-ready' });
  const sp = prSplitProposal(splitRead());
  assert.strictEqual(sp.key, 'top:split');
  assert.ok(sp.html.includes('PR1: split\nPR2: &lt;b&gt;rest&lt;/b&gt;') && !sp.html.includes('<b>rest') && sp.html.includes('下書きタブで編集・コピー・pending review に積めます。'));
  assert.strictEqual(prSplitProposal(read()), null);
  assert.strictEqual(prSplitProposal(null), null);
  assert.deepStrictEqual(secKeys(prDetailHtml(r, splitRead(), [r], hub, T0)), ['top:facts', 'top:oor', 'top:three', 'top:split', 'footer']);
  const groups = prDialogGroups(r, splitRead(), [r], hub, T0);
  assert.deepStrictEqual(plain(groups.map(g => g.key)), ['top']);
  assert.deepStrictEqual(plain(groups[0].cards.map(c => c.key)), ['top:facts', 'top:oor', 'top:three', 'top:split', 'footer']);
  // A full read-through of the same PR keeps its diff group, and the card of the files over the limit.
  const full = prDialogGroups(big({ ai: { status: 'ready', sha: A, stale: false }, state: 'ai-ready' }), read(), [], hub, T0);
  assert.ok(full.some(g => g.key === 'diff') && full[0].cards.some(c => c.key === 'top:oor'));
  // 差分 is disabled for a proposal, with the reason.
  const tabs = prTabsHtml(r, splitRead(), 'detail', 0, true);
  assert.ok(tabs.includes('分割案では差分を読んでいません'));
});

test('the overall comment of a read that the person\'s file lacks is offered, and one it holds is not', () => {
  const doc = { drafts: [], overall: { body: '' } };
  assert.strictEqual(prNewReaderOverall(doc, splitRead()), 'PR1: split\nPR2: <b>rest</b>');
  assert.strictEqual(prNewReaderOverall(null, splitRead()), 'PR1: split\nPR2: <b>rest</b>');
  assert.strictEqual(prNewReaderOverall({ overall: { body: 'mine\r\n\nPR1: split\nPR2: <b>rest</b>\n' } }, splitRead()), null);
  assert.strictEqual(prNewReaderOverall({ overall: { body: 'mine' } }, splitRead()), 'PR1: split\nPR2: <b>rest</b>');
  assert.strictEqual(prNewReaderOverall(doc, read({ overall: { body: '  ' } })), null);
  assert.strictEqual(prNewReaderOverall(doc, read({ overall: null })), null);
  // A plain re-read does not start offering a second summary.
  assert.strictEqual(prNewReaderOverall(doc, read({ overall: { body: 'another summary' } })), null);
  // Until the first edit the proposal stands as the overall comment of the seed.
  assert.strictEqual(prSeedDrafts(splitRead()).overall.body, 'PR1: split\nPR2: <b>rest</b>');
  assert.deepStrictEqual(plain(prSeedDrafts(splitRead()).drafts), []);
});

test('drafts were taken from another read-through when the head or the mode differs', () => {
  const f = (doc, rd) => prFromElsewhere(plain(doc), rd);
  assert.strictEqual(f({ fromSha: A }, read()), false);
  assert.strictEqual(f({ fromSha: B }, read()), true);
  // Same head, other mode, either way; a file from before the mode is a read.
  assert.strictEqual(f({ fromSha: A, fromMode: 'split' }, read()), true);
  assert.strictEqual(f({ fromSha: A }, splitRead()), true);
  assert.strictEqual(f({ fromSha: A, fromMode: 'split' }, splitRead()), false);
  assert.strictEqual(prSeedDrafts(splitRead()).fromMode, 'split');
  assert.ok(!('fromMode' in plain(prSeedDrafts(read()))));
});

test('the footer asks for a read in the usual words, and a PR out of review leaves it to its card', () => {
  const stale = rec({ headSha: B, ai: { status: 'ready', sha: A, mode: 'split', stale: true } });
  const footer = html => html.slice(html.indexOf('data-pr-sec="footer"'));
  // A proposal of a PR that is small now: the usual button.
  const f = footer(prDetailHtml(stale, splitRead(), [stale], hub, T0));
  assert.ok(f.includes('data-pr-reread') && f.includes('下読みし直す') && !f.includes('分割を提案し直す'));
  for (const [ai, rd] of [[{ status: 'ready', sha: A, mode: 'split', stale: true }, splitRead()], [{ status: 'ready', sha: A, stale: true }, read()]]) {
    const r = big({ headSha: B, ai, state: 'ai-ready' });
    const html = prDetailHtml(r, rd, [r], hub, T0);
    assert.ok(!footer(html).includes('data-pr-reread') && html.includes('data-pr-split'));
  }
});

test('a proposal that failed or whose reader hung is offered again even when the stored one is current', () => {
  const stored = splitRead();
  for (const status of ['failed', 'running']) {
    const r = big({ ai: { status, mode: 'split', sha: A, stale: false, launchedAt: '20260101T000000Z', error: 'x' }, state: 'ai-ready' });
    const view = plain(ctx.othersAiView(r, [r], T0));
    assert.deepStrictEqual(plain(prOorActions(r, stored, view)).split, '分割を提案し直す', status);
  }
});

test('the footer offers the usual read for a current proposal of a PR that is not out of review now, and none while it is', () => {
  const proposal = rec({ ai: { status: 'ready', sha: A, mode: 'split', stale: false } });
  const footer = html => html.slice(html.indexOf('data-pr-sec="footer"'));
  const f = footer(prDetailHtml(proposal, splitRead(), [proposal], hub, T0));
  assert.ok(f.includes('data-pr-reread') && f.includes('下読みする'));
  const r = big({ ai: { status: 'ready', sha: A, mode: 'split', stale: false }, state: 'ai-ready' });
  assert.ok(!footer(prDetailHtml(r, splitRead(), [r], hub, T0)).includes('data-pr-reread'));
  // A full read-through that is current has no such button.
  assert.ok(!footer(prDetailHtml(rec(), read(), [rec()], hub, T0)).includes('data-pr-reread'));
});

test('a read-through that cannot be fetched does not hide the buttons of a PR out of review', () => {
  const r = big({ ai: { status: 'ready', sha: A, stale: true }, state: 'ai-ready' });
  const view = plain(ctx.othersAiView(r, [r], T0));
  assert.deepStrictEqual(plain(prOorActions(r, undefined, view)), { split: null, full: null });
  assert.deepStrictEqual(plain(prOorActions(r, undefined, view, true)), { split: '分割を提案する', full: 'それでも AI 下読み' });
  assert.ok(prDetailHtml(r, undefined, [r], hub, T0, { readError: 'boom' }).includes('data-pr-split'));
  assert.ok(prMiddleHtml(r, [r], hub, T0, { read: undefined, readError: true }).includes('data-pr-split'));
});

const open = (extra = {}) => rec({ ai: { status: 'running', sha: null, launchedAt: '20261001T000000Z', sessionId: 's1', pid: 4242, terminal: { backend: 'tmux', window: '@3' }, ...extra } });

test('「下読みを閉じる」 is offered only while a reader runs that has a process', () => {
  const closes = r => prCloseReaderBtnHtml(r, false).includes('data-pr-close-reader');
  assert.ok(closes(open()) && prCloseReaderBtnHtml(open(), false).includes('下読みを閉じる'));
  assert.ok(closes(rec({ ai: { ...open().ai, status: 'ready' } })));
  // No reader, no window, no process yet, a closed one, and a read that failed or waits.
  assert.ok(!closes(rec({ ai: null })));
  assert.ok(!closes(rec()));
  assert.ok(!closes(open({ pid: undefined })));
  assert.ok(!closes(open({ readerClosedAt: '20261001T000009Z' })));
  assert.ok(!closes(open({ status: 'failed' })));
  assert.ok(!closes(open({ status: 'queued' })));
  assert.strictEqual(run('prReaderOpen')(open({ readerClosedAt: 'x' })), false);
});

test('a PR out of review reconnects a reader that ended or could not be reached, and says each thing once', () => {
  const split = big({ ai: { status: 'running', mode: 'split', sessionId: 's', pid: 1, terminal: { backend: 'tmux', window: '@3' } }, state: 'requested' });
  const failed = prMiddleHtml(split, [split], hub, T0, { failed: 'セッションが見つかりません: x' });
  assert.ok(failed.includes('data-pr-reconnect') && failed.includes('worker に接続できませんでした'));
  assert.strictEqual(failed.split('セッションが見つかりません: x').length - 1, 1);
  assert.ok(!failed.includes('worker に接続できませんでした:'));
  const ended = prMiddleHtml(split, [split], hub, T0, { ended: 4410 });
  assert.ok(ended.includes('data-pr-reconnect') && ended.includes('worker は終わっています'));
  assert.ok(!prMiddleHtml(split, [split], hub, T0, {}).includes('data-pr-reconnect'));
  const closed = { ...split, ai: { ...split.ai, readerClosedAt: '20261001T000009Z' } };
  assert.ok(!prMiddleHtml(closed, [closed], hub, T0, { failed: 'x' }).includes('data-pr-reconnect'));
});

test('再接続 is offered only while the reader can be mounted, and a reason that repeats the line over it is not said twice', () => {
  const live = big({ ai: { status: 'running', mode: 'split', sessionId: 's', pid: 1, terminal: { backend: 'tmux', window: '@3' } }, state: 'requested' });
  const dead = { ...live, ai: { ...live.ai, status: 'failed' } };
  for (const r of [live, dead]) {
    assert.strictEqual(prMiddleHtml(r, [r], hub, T0, { ended: 4410 }).includes('data-pr-reconnect'), r === live);
    assert.strictEqual(prMiddleHtml(r, [r], hub, T0, { failed: 'x' }).includes('data-pr-reconnect'), r === live);
  }
  const plain = rec({ ai: { status: 'ready', sha: A, sessionId: 's', pid: 1, terminal: { backend: 'tmux', window: '@3' } } });
  const gone = rec({ ai: { status: 'failed', sha: A, sessionId: 's', pid: 1, terminal: { backend: 'tmux', window: '@3' } } });
  assert.ok(prMiddleHtml(plain, [plain], hub, T0, { ended: 4410 }).includes('data-pr-reconnect'));
  assert.ok(!prMiddleHtml(gone, [gone], hub, T0, { ended: 4410 }).includes('data-pr-reconnect'));
  const same = prMiddleHtml(plain, [plain], hub, T0, { failed: '接続できませんでした' });
  assert.strictEqual(same.split('接続できませんでした').length - 1, 1);
  assert.ok(prMiddleHtml(plain, [plain], hub, T0, { failed: 'boom' }).includes('boom'));
});

test('the middle of a closed reader says so and offers 「下読みし直す」, and wins over an ended terminal', () => {
  const closed = open({ status: 'ready', readerClosedAt: '20261001T000009Z' });
  const html = prMiddleHtml(closed, [closed], hub, T0);
  assert.ok(html.includes('AI 下読みの worker を閉じました') && html.includes('下読みは真ん中に残っています。'));
  assert.match(html, /data-pr-ai[^>]*>.*下読みし直す/s);
  const ended = prMiddleHtml(closed, [closed], hub, T0, { ended: 4404 });
  assert.ok(ended.includes('AI 下読みの worker を閉じました') && !ended.includes('data-pr-reconnect') && !ended.includes('終わっています'));
  assert.ok(prMiddleHtml(closed, [closed], { canRead: false, why: 'stopped' }, T0).includes('disabled title="stopped"'));
  assert.ok(prMiddleHtml(closed, [closed], hub, T0, { asking: true }).includes('依頼中…'));
  // A PR that left review has nothing to ask again.
  const done = { ...closed, state: 'done' };
  const gone = prMiddleHtml(done, [done], hub, T0);
  assert.ok(gone.includes('AI 下読みの worker を閉じました') && !gone.includes('data-pr-ai'));
  // Not closed: the usual words.
  assert.ok(!prMiddleHtml(rec(), [rec()], hub, T0, { ended: 4404 }).includes('を閉じました'));
});

test('「聞く」 is disabled while no reader runs', () => {
  const on = prDetailHtml(open(), read(), [open()], hub, T0);
  assert.ok(/data-pr-ask="0">聞く/.test(on) && !/data-pr-ask="0" disabled/.test(on));
  for (const r of [rec(), open({ readerClosedAt: '20261001T000009Z' }), rec({ ai: null })]) {
    const off = prDetailHtml(r, read(), [r], hub, T0);
    assert.match(off, /data-pr-ask="0" disabled title="worker が動いていません">聞く/);
    assert.match(off, /data-pr-ask="1" disabled/);
  }
});

test('a closed reader of a PR out of review is said in its middle too, with the split buttons kept', () => {
  const closed = big({ ai: { status: 'failed', mode: 'split', error: 'the reader was closed', readerClosedAt: '20261001T000009Z' }, state: 'requested' });
  const html = prMiddleHtml(closed, [closed], hub, T0);
  assert.ok(html.includes('レビュー対象外') && html.includes('AI 下読みの worker を閉じました') && html.includes('data-pr-split'));
  assert.ok(!prMiddleHtml(big(), [big()], hub, T0).includes('を閉じました'));
});

test('「下読みを閉じる」 is disabled while this page\'s close is out', () => {
  assert.match(prCloseReaderBtnHtml(open(), true), /data-pr-close-reader disabled/);
  assert.ok(!/data-pr-close-reader disabled/.test(prCloseReaderBtnHtml(open(), false)));
  assert.ok(!/data-pr-close-reader disabled/.test(prCloseReaderBtnHtml(open())));
});

test('the close button and the stale line are drawn the same where a task reuses them', () => {
  const prStaleText = run('prStaleText');
  assert.strictEqual(prCloseReaderBtnHtml(open({ pid: undefined }), false), '');
  assert.strictEqual(prCloseReaderBtnHtml(rec({ ai: null }), false), '');
  assert.match(prCloseReaderBtnHtml(open(), true), /data-pr-close-reader disabled[^>]*>閉じています…</);
  assert.strictEqual(prStaleText({ sha: A }, { headSha: B }), `この差分は ${'a'.repeat(7)} で下読みしたときのものです（PR の head は ${'b'.repeat(7)}）`);
});

test('the placeholder over the reader says the read-through is in the middle', () => {
  const closed = open({ status: 'ready', readerClosedAt: '20261001T000009Z' });
  assert.ok(prMiddleHtml(closed, [closed], hub, T0).includes('下読みは真ん中に残っています。'));
  assert.ok(!prMiddleHtml(closed, [closed], hub, T0).includes('パネル'));
  const ended = prMiddleHtml(rec(), [rec()], hub, T0, { ended: 4404 });
  assert.ok(ended.includes('下読みは真ん中に残っています。') && !ended.includes('パネル'));
  const done = rec();
  assert.ok(prMiddleHtml(done, [done], hub, T0).includes('下読みは真ん中にあります。worker の画面は開けません。'));
});

test('the card before a read names where the reader can be talked to, when it is asked to', () => {
  const r = rec({ ai: null, state: 'requested' });
  assert.ok(!prBeforeRead(r, [r], hub, T0).html.includes('聞けます'));
  assert.ok(prBeforeRead(r, [r], hub, T0, false, '右のターミナル').html.includes('読んだあとは、右のターミナルでその worker に差分のことを聞けます。'));
  // What a read under way says is not added to.
  const q = rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' }, state: 'ai-reading' });
  assert.ok(!prBeforeRead(q, [q], hub, T0, false, '右のターミナル').html.includes('聞けます'));
});

/* ── the drafts of a task's PR go to its worker ── */

const handDoc = () => ({ drafts: [
  { key: 'r1', source: 'reader', path: 'a.rs', line: 1, body: 'one' }, { key: 'p1', source: 'person', body: 'whole PR' },
  { key: 'b', source: 'person', path: 'b.rs', body: '  ' }, { key: 's', source: 'person', path: 'c.rs', line: 2, body: 'sent', sentAt: '20261012T000000Z' }], overall: { body: 'all' } });

test('what can be handed to the worker: the drafts with something written that are not sent (a draft with no file too), and a written overall comment', () => {
  assert.deepStrictEqual(plain(prHandable(handDoc())), { drafts: 2, overall: true });
  assert.deepStrictEqual(plain(prHandable({ drafts: [], overall: { body: ' \n' } })), { drafts: 0, overall: false });
  assert.deepStrictEqual(plain(prHandable({ drafts: [], overall: { body: 'x' } })), { drafts: 0, overall: true });
  assert.deepStrictEqual(plain(prHandable(null)), { drafts: 0, overall: false });
  // Not the pending review's rule: a draft with no file counts here, and a note of an overall comment already in a review does not hide it.
  assert.deepStrictEqual(plain(prPendable(handDoc())), { drafts: 1, overall: true });
  assert.deepStrictEqual(plain(prHandable({ drafts: [], overall: { body: 'x' }, pending: { overall: 'x' } })), { drafts: 0, overall: true });
});

test('drafts for the worker: 渡しました for what was sent, the button counts what is left, and there is no pending review', () => {
  const html = prDraftsHtml(handDoc(), read(), { to: 'worker', hand: {} });
  assert.ok(html.includes('class="pr-chip pr-chip-sent" title="worker に渡した指示です">渡しました</span>'));
  assert.ok(!html.includes('積みました') && !/data-pr-pend[ >]/.test(html) && !html.includes('pending review に積む') && !html.includes('GitHub で仕上げる'));
  assert.ok(/<button type="button" class="btn-m3-filled" data-pr-hand>2 件を worker に渡す（全体のコメントも）<\/button>/.test(html), html);
  // What was sent is read-only; AI marks the reader's.
  const [first, , , sent] = html.split('data-pr-draft="').slice(1);
  assert.ok(first.includes('title="AI 下読みの下書きです">AI</span>') && !first.includes('渡しました'));
  assert.ok(sent.includes('渡しました</span>') && /readonly>/.test(sent) && sent.includes('data-pr-copy="s"'));
  assert.ok(html.includes('data-pr-pending-slot'));
  // A review request's drafts are as they were.
  const pr = prDraftsHtml(handDoc(), read(), { pend: { prUrl: rec().url } });
  assert.ok(/data-pr-pend[ >]/.test(pr) && pr.includes('積みました</span>') && !pr.includes('data-pr-hand') && !pr.includes('渡しました'));
  assert.ok(prDraftsHtml(handDoc(), read(), {}).includes('積みました</span>'), 'the default is the pending review');
});

test('the hand-over button: only the comment, nothing to send (disabled with the reason), busy, and how the last one went', () => {
  const only = prHandHtml({ drafts: [], overall: { body: 'x' } }, {});
  assert.ok(/data-pr-hand>全体のコメントを worker に渡す<\/button>/.test(only), only);
  const none = prHandHtml({ drafts: [], overall: { body: '' } }, {});
  assert.ok(/data-pr-hand disabled title="[^"]+">worker に渡す<\/button>/.test(none), none);
  assert.ok(none.includes('<div class="pr-pend-note">渡せるものがありません。</div>'));
  const html = prHandHtml(handDoc(), {});
  assert.ok(!html.includes('渡せるものがありません'));
  // Where it goes: a waiting diff or verify is answered with 差し戻す, else the latest diff record.
  assert.ok(html.includes('差し戻す') && html.includes('最新の差分の記録'));
  const busy = prHandHtml(handDoc(), { sending: true });
  assert.ok(busy.includes('data-pr-hand disabled aria-busy="true"') && busy.includes('渡しています…'));
  assert.ok(prHandHtml(handDoc(), { result: { sent: ['a', 'b'], overall: true } }).includes('role="status">2 件を渡しました／全体のコメントも渡しました</div>'));
  assert.ok(prHandHtml(handDoc(), { result: { sent: ['a'], overall: false } }).includes('1 件を渡しました</div>'));
  // A refusal is said as text, never as markup.
  const refused = prHandHtml(handDoc(), { result: { error: '<b>no</b>' } });
  assert.ok(refused.includes('role="note">&lt;b&gt;no&lt;/b&gt;</div>') && !refused.includes('<b>no'));
});

test('everything is locked while the drafts are on their way to the worker', () => {
  const html = prDraftsHtml(handDoc(), read(), { to: 'worker', notice: 1, hand: { sending: true } });
  assert.ok(/class="pr-overall"[^>]* readonly>/.test(html) && /data-pr-discard="r1" disabled/.test(html) && html.includes('data-pr-take-new disabled'));
  assert.strictEqual((html.match(/<textarea class="pr-draft-body"[^>]*readonly>/g) || []).length, 4);
  // The pending review's `sending` is not the worker's.
  const other = prDraftsHtml(handDoc(), read(), { to: 'worker', hand: {}, pend: { sending: true } });
  assert.ok(!/class="pr-overall"[^>]* readonly>/.test(other));
});
