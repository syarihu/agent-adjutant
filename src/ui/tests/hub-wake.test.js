/* The rules of the hub's wake button (hubWakeBlocked, src/ui/actions.js), run with `node --test`. The rules are the same wherever the
   button is drawn: the board's hub entry, the hub panel and the chip in 「いまの仕事」. */
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
const panelSrc = fs.readFileSync(path.join(__dirname, '..', 'task-panel.js'), 'utf8');
const cutPanel = re => {
  const m = panelSrc.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const posts = [];
const ctx = vm.createContext({
  BASE: '/b/own', multiBoard: true, boards: [{ slug: 'own' }, { slug: 'other' }], navEpoch: 0, note() {}, refresh: async () => {},
  redrawHubWake() {}, pageHub: () => ({ id: 'own', slug: 'own' }), document: { activeElement: null, body: {}, querySelectorAll: () => [] },
  CSS: { escape: x => x }, boardApi: async (base, p, o) => { posts.push([base, p, o.method]); return { woken: true }; },
  esc: x => String(x),
});
vm.runInContext([
  cut(/^const hubWake = [^\n]*;/m),
  cut(/^const HUB_WAKE_TITLE = [^\n]*;/m),
  cut(/^async function wakeHub[\s\S]*?^}/m),
  cutPanel(/^const hubOther = [^\n]*;/m),
  cutPanel(/^const hubRowOf = [^\n]*;/m),
  cutPanel(/^const hubBaseOf = [^\n]*;/m),
  cutPanel(/^const hubAway = [^\n]*;/m),
  cutPanel(/^const hubPanelWakeBase = [^\n]*;/m),
  cut(/^function hubWakeBlocked[\s\S]*?^}/m),
  cut(/^function hubWakeWhy[\s\S]*?^}/m),
].join('\n'), ctx);
const blocked = (h, base) => { ctx.h = h; ctx.base = base; return vm.runInContext('hubWakeBlocked(h, base)', ctx); };
const hub = (extra = {}) => ({ slug: 'a', unseen: 1, state: { present: true }, ...extra });

test('a running hub with something unseen and a board to ask can be woken', () => {
  assert.strictEqual(blocked(hub(), '/b/a'), '');
  // A board served on its own is asked at the root.
  assert.strictEqual(blocked(hub(), ''), '');
});

test('the reasons come in the order of the board: nothing unseen, no board here, stopped, on its way', () => {
  const stopped = hub({ state: { present: false }, unseen: 0 });
  assert.strictEqual(blocked(stopped, null), '未確認のメッセージはありません');
  assert.strictEqual(blocked(hub({ unseen: undefined }), '/b/a'), '未確認のメッセージはありません');
  assert.strictEqual(blocked(hub({ state: { present: false } }), null), 'この hub のボードはこのサーバーにありません');
  assert.strictEqual(blocked(hub({ state: { present: false } }), '/b/a'), 'hub が止まっています');
  assert.strictEqual(blocked(hub({ state: undefined }), '/b/a'), 'hub が止まっています');
  vm.runInContext('hubWake.busy.a = true', ctx);
  assert.strictEqual(blocked(hub(), '/b/a'), '起こしています');
  // A press on one board leaves another's button alone.
  assert.strictEqual(blocked(hub({ slug: 'b' }), '/b/b'), '');
});

test('wakeHub posts to the base it is given, and does nothing while blocked', async () => {
  posts.length = 0;
  await vm.runInContext('wakeHub({ slug: "other", unseen: 1, state: { present: true } }, "/b/other")', ctx);
  assert.deepStrictEqual(posts, [['/b/other', '/api/hub/wake', 'POST']]);
  posts.length = 0;
  await vm.runInContext('wakeHub({ slug: "other", unseen: 0, state: { present: true } }, "/b/other")', ctx);
  await vm.runInContext('wakeHub({ slug: "other", unseen: 1, state: { present: true } }, null)', ctx);
  await vm.runInContext('wakeHub({ slug: "other", unseen: 1, state: { present: false } }, "/b/other")', ctx);
  assert.deepStrictEqual(posts, []);
});

test('the panel wakes the hub on its own board: the page\'s, another served one, or none when this server lacks it', () => {
  const base = h => { ctx.h = h; return vm.runInContext('hubPanelWakeBase(h)', ctx); };
  // `hubOther` compares with the page's hub; another board's is counted by its row in the board list.
  assert.strictEqual(base({ id: 'own', slug: 'own' }), '/b/own');
  assert.strictEqual(base({ id: 'o2', slug: 'other' }), '/b/other');
  assert.strictEqual(base({ id: 'x', slug: 'gone' }), null);
});

test('the hub panel draws the wake button in its 受信箱 card, enabled for an unseen message and off for none', () => {
  const names = ['hubStartWhy', 'hubWaitWhy', 'sessionPendingRows', 'sinceLabel', 'secTitle', 'kv', 'agentFactsHtml', 'waitingIn', 'hubListMore', 'hubActionOf', 'canRestart', 'restartWhy', 'idleWorktreesHtml', 'when', 'pendingRowHtml'];
  for (const n of names) ctx[n] = n === 'sessionPendingRows' ? () => [] : () => '';
  Object.assign(ctx, { state: { tasks: [], resident: true }, HUB_LIST_MAX: 5, NO_START: 'x', hubActionOf: () => null });
  vm.runInContext(cutPanel(/^function hubDetailHtml[\s\S]*?^}/m), ctx);
  const html = h => { ctx.h = h; return vm.runInContext('hubDetailHtml(h, { present: true })', ctx); };
  const hub = (extra = {}) => ({ id: 'own', slug: 'own', unseen: 1, seen: 0, inbox: [], inboxCount: 1, state: { present: true }, ...extra });
  const on = html(hub());
  assert.match(on, /data-tp-hub="wake"(?! disabled)[^>]*title="hub の端末/);
  assert.match(html(hub({ unseen: 0, seen: 1 })), /data-tp-hub="wake" disabled title="未確認のメッセージはありません"/);
  assert.ok(!html(hub({ unseen: 0, seen: 0 })).includes('data-tp-hub="wake"'));
});
