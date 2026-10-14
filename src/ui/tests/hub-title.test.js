/* Where a parent-task hub's title stands and the button that reads its issue now (hubTitleWords, hubTitleReadBlocked and
   readHubTitle in src/ui/actions.js), run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'actions.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const posts = [];
const refreshed = [];
let answer = () => ({ title: 'T' });
const ctx = vm.createContext({
  navEpoch: 0, note() {}, refresh: async force => { refreshed.push(force); }, redrawHubWake() {},
  document: { activeElement: null, body: {}, querySelectorAll: () => [] }, CSS: { escape: x => x },
  boardApi: async (base, p, o) => { posts.push([base, p, o.method]); return answer(); },
  esc: x => String(x).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;'),
  pad2: n => String(n).padStart(2, '0'), when: x => `W(${x})`,
});
vm.runInContext([
  cut(/^const hubTitleRead = [^\n]*;/m),
  cut(/^const HUB_TITLE_READ_LABEL = [^\n]*;/m),
  cut(/^const hubTitleClock = [^\n]*;/m),
  cut(/^function hubTitleWords[\s\S]*?^}/m),
  cut(/^function hubTitleMarkHtml[\s\S]*?^}/m),
  cut(/^function hubTitleReadBlocked[\s\S]*?^}/m),
  cut(/^function hubTitleReadButtonHtml[\s\S]*?^}/m),
  cut(/^function hubTitleReadWhy[\s\S]*?^}/m),
  cut(/^function hubTitleWhyHtml[\s\S]*?^}/m),
  cut(/^async function readHubTitle[\s\S]*?^}/m),
].join('\n'), ctx);
const call = (expr, vars = {}) => { Object.assign(ctx, vars); return vm.runInContext(expr, ctx); };
const hub = titleState => ({ slug: 'a', key: 'ALPHA-9', parent: true, titleState });
const words = (titleState) => call('hubTitleWords(h)', { h: hub(titleState) });

test('a hub with no title state, or a title read without counts, has nothing to say', () => {
  assert.strictEqual(call('hubTitleWords(h)', { h: { slug: 'a', parent: false } }), null);
  assert.strictEqual(words({ status: 'read', reading: false }), null);
});

test('pending says whether the title is being read', () => {
  assert.strictEqual(words({ status: 'pending', reading: true }).tip, 'タイトルを読み込み中');
  assert.strictEqual(words({ status: 'pending', reading: false }).tip, 'タイトルはまだ読み込んでいません');
});

test('a failed read says why and when it is tried again', () => {
  const retryAt = 1_800_000_000;
  const d = new Date(retryAt * 1000);
  const hhmm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  assert.strictEqual(words({ status: 'failed', why: 'rate limited', retryAt }).tip,
    `タイトルを読めませんでした: rate limited。${hhmm} にもう一度読みます`);
  assert.strictEqual(words({ status: 'failed' }).tip, 'タイトルを読めませんでした: 理由不明');
});

test('a hub with no issue says which of the three ways it has none', () => {
  assert.strictEqual(words({ status: 'missing', missing: 'noKey' }).tip, 'この hub のキーが分かりません');
  assert.strictEqual(words({ status: 'missing', missing: 'noIssue' }).tip, 'キー ALPHA-9 に当たる親 Issue がありません');
  assert.strictEqual(words({ status: 'missing', missing: 'notFound', parentKey: 'acme/widget#9' }).tip,
    '親 Issue acme/widget#9 が GitHub に見つかりません');
});

test('a read title shows the sub-issue progress, with when it was read in the tooltip', () => {
  const w = words({ status: 'read', total: 5, completed: 2, readAt: '20261014T010203Z' });
  assert.strictEqual(w.text, 'サブ Issue 完了 2 / 5');
  assert.strictEqual(w.tip, 'サブ Issue 完了 2 / 5（W(20261014T010203Z) に読み込み）');
  // A heading has its own counter, so its mark is not drawn for a read title.
  const read = hub({ status: 'read', total: 5, completed: 2 });
  assert.strictEqual(call('hubTitleMarkHtml(h, false)', { h: read }), '');
  assert.match(call('hubTitleMarkHtml(h)', { h: read }), /role="img"[^>]*>サブ Issue 完了 2 \/ 5</);
});

test('the mark is an image with a name, and its glyph is decorative', () => {
  const html = call('hubTitleMarkHtml(h)', { h: hub({ status: 'failed', why: '<b>' }) });
  assert.match(html, /role="img" aria-label="[^"]*&lt;b>/);
  assert.match(html, /aria-hidden="true">error</);
});

test('the reasons the issue cannot be read from here', () => {
  const blocked = (h, base) => call('hubTitleReadBlocked(h, base)', { h, base });
  assert.strictEqual(blocked(hub({ status: 'pending' }), '/b/a'), '');
  // A board served on its own is asked at the root.
  assert.strictEqual(blocked(hub({ status: 'pending' }), ''), '');
  assert.strictEqual(blocked(hub({ status: 'pending' }), null), 'この hub のボードはこのサーバーにありません');
  assert.strictEqual(blocked(hub({ status: 'missing', missing: 'noKey' }), '/b/a'), '読む親 Issue がありません');
  assert.strictEqual(blocked(hub({ status: 'missing', missing: 'noIssue' }), '/b/a'), '読む親 Issue がありません');
  // An issue GitHub does not have may have been made since: it can be asked again.
  assert.strictEqual(blocked(hub({ status: 'missing', missing: 'notFound' }), '/b/a'), '');
  // A read the server says is out (a background one) blocks as well as one this page sent.
  assert.strictEqual(blocked(hub({ status: 'pending', reading: true }), '/b/a'), '読み込んでいます');
  call('hubTitleRead.busy.a = true');
  assert.strictEqual(blocked(hub({ status: 'pending' }), '/b/a'), '読み込んでいます');
  // A press on one hub leaves another's button alone.
  assert.strictEqual(blocked({ ...hub({ status: 'pending' }), slug: 'b' }, '/b/b'), '');
  call('delete hubTitleRead.busy.a');
});

test('the button is disabled with the reason, and is not drawn for a hub with no title state', () => {
  const button = (h, base) => call('hubTitleReadButtonHtml(h, base, "data-x=1")', { h, base });
  assert.strictEqual(button({ slug: 'a', parent: false }, '/b/a'), '');
  const on = button(hub({ status: 'pending' }), '/b/a');
  assert.match(on, /<button type="button" class="hub-title-read" data-hub-title-read="a" data-x=1 aria-label="親 Issue をいま読み込む"/);
  assert.ok(!/ disabled/.test(on));
  assert.match(button(hub({ status: 'pending' }), null), / disabled aria-label=.*title="この hub のボードはこのサーバーにありません"/);
});

test('readHubTitle posts to the base it is given, once at a time, and says what it was told', async () => {
  posts.length = 0;
  refreshed.length = 0;
  const h = hub({ status: 'pending' });
  const first = call('readHubTitle(h, "/b/a")', { h });
  // The second press, while the first is on its way, sends nothing.
  await call('readHubTitle(h, "/b/a")', { h });
  await first;
  assert.deepStrictEqual(posts, [['/b/a', '/api/hub/title', 'POST']]);
  assert.deepStrictEqual(refreshed, [true]);
  assert.strictEqual(call('hubTitleRead.busy.a'), undefined);

  posts.length = 0;
  await call('readHubTitle(h, null)', { h });
  assert.deepStrictEqual(posts, []);

  // A failure is said on the heading, until the issue has been read since.
  answer = () => { throw new Error('boom'); };
  await call('readHubTitle(h, "/b/a")', { h });
  assert.strictEqual(call('hubTitleReadWhy(h)', { h }), '読み込めませんでした: boom');
  assert.match(call('hubTitleWhyHtml(h)', { h }), /role="status">読み込めませんでした: boom</);
  const later = hub({ status: 'read', total: 1, completed: 0, readAt: '20261014T000000Z' });
  assert.strictEqual(call('hubTitleReadWhy(h)', { h: later }), '');
  answer = () => ({ title: 'T' });
});

test('a GitHub that has no such issue is said with the issue named', async () => {
  answer = () => ({ missing: 'notFound' });
  const h = hub({ status: 'pending', parentKey: 'acme/widget#9' });
  await call('readHubTitle(h, "/b/a")', { h });
  assert.strictEqual(call('hubTitleReadWhy(h)', { h }), '親 Issue acme/widget#9 が GitHub に見つかりません');
  answer = () => ({ title: 'T' });
});

test('a reply that another read is out is not shown as a failure', async () => {
  answer = () => ({ busy: true });
  const h = hub({ status: 'pending' });
  await call('readHubTitle(h, "/b/a")', { h });
  assert.strictEqual(call('hubTitleReadWhy(h)', { h }), '');
  answer = () => ({ title: 'T' });
});
