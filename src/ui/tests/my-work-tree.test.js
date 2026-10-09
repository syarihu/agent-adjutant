/* The owner level and the 状態 boxes' repository level of 「いまの仕事」 (src/ui/my-work.js), run with `node --test`.
   my-work.js draws into the page as it loads, so only the pure helpers are cut out of the source and evaluated here. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'my-work.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
// STATE_ORDER and stampSecs live in other files of the page; a stub is enough for the ordering the rows here need.
const ctx = vm.createContext({ STATE_ORDER: { working: 0, waiting: 1, done: 2 }, stampSecs: () => 0 });
vm.runInContext([
  cut(/^const WORK_BOXES = [\s\S]*?^\];/m),
  cut(/^const WORK_RUNNING = [^\n]*;/m),
  cut(/^const workBoxOf = [^\n]*;/m),
  cut(/^function workRowOrder[\s\S]*?^}/m),
  cut(/^const workStateOrder = [^\n]*;/m),
  cut(/^const WORKS_ON_PERSON = [^\n]*;/m),
  cut(/^const workOwnerName = [^\n]*;/m),
  cut(/^function workOwnerGroups[\s\S]*?^}/m),
  cut(/^function workBoxRepoGroups[\s\S]*?^}/m),
  cut(/^function workTreeByState[\s\S]*?^}/m),
].join('\n'), ctx);
const plain = v => JSON.parse(JSON.stringify(v));
ctx.input = null;
const groups = nodes => { ctx.input = nodes; return vm.runInContext('workOwnerGroups(input)', ctx); };
const name = nwo => { ctx.arg = nwo; return vm.runInContext('workOwnerName(arg)', ctx); };
const repo = (nwo, n = 1) => ({ key: `repo:${nwo}`, kind: 'repo', nwo, rows: Array.from({ length: n }, (_, i) => ({ key: `${nwo}#${i}` })), items: [] });

test('the owner is the part of the name before the slash, or nothing', () => {
  assert.strictEqual(name('acme/widget'), 'acme');
  for (const v of ['widget', '/x', '', null]) assert.strictEqual(name(v), '');
});

test('repositories are grouped under their owners, in input order, with their rows', () => {
  const a = repo('acme/a', 2), b = repo('acme/b'), c = repo('zed/c');
  const out = groups([a, b, c]);
  assert.deepStrictEqual(plain(out.map(o => o.key)), ['owner:acme', 'owner:zed']);
  assert.deepStrictEqual(plain(out[0].items.map(i => i.nwo)), ['acme/a', 'acme/b']);
  assert.strictEqual(out[0].items[0], a);
  assert.strictEqual(out[0].rows.length, 3);
  assert.strictEqual(out[0].rows[0], a.rows[0]);
  assert.strictEqual(out[0].rows[2], b.rows[0]);
  assert.strictEqual(out[0].kind, 'owner');
});

test('a single owner still gets an owner node', () => {
  const out = groups([repo('acme/a')]);
  assert.strictEqual(out.length, 1);
  assert.strictEqual(out[0].key, 'owner:acme');
});

test('owners are in name order, case ignored', () => {
  assert.deepStrictEqual(plain(groups([repo('Zeta/x'), repo('acme/y')]).map(o => o.label)), ['acme', 'Zeta']);
});

test('owners that differ only in case are one node, labelled by the first spelling', () => {
  const out = groups([repo('Acme/x'), repo('acme/y')]);
  assert.strictEqual(out.length, 1);
  assert.strictEqual(out[0].key, 'owner:acme');
  assert.strictEqual(out[0].label, 'Acme');
  assert.strictEqual(out[0].items.length, 2);
});

test('a name with no slash goes last under オーナーなし', () => {
  const out = groups([repo('local'), repo('acme/a')]);
  assert.deepStrictEqual(plain(out.map(o => o.key)), ['owner:acme', 'owner:']);
  assert.strictEqual(out[1].label, 'オーナーなし');
});

test('owner keys do not clash with repo or org keys', () => {
  for (const o of groups([repo('acme/a'), repo('local')])) assert.ok(o.key.startsWith('owner:'));
});

const boxGroups = (box, rows, place = 'box') => { ctx.b = box; ctx.r = rows; ctx.p = place; return vm.runInContext('workBoxRepoGroups(b, r, p)', ctx); };
const row = (nwo, id) => ({ key: `${nwo}#${id}`, nwo });

test('a box has a heading per repository with the full owner/name, and no owner level', () => {
  const out = boxGroups('running', [row('acme/a', 1), row('acme/b', 2)]);
  assert.deepStrictEqual(plain(out.map(g => g.label)), ['acme/a', 'acme/b']);
  for (const g of out) assert.strictEqual(g.kind, 'brepo');
});

test('box headings are in name order with case ignored', () => {
  const out = boxGroups('running', [row('Zeta/x', 1), row('acme/y', 2), row('acme/B', 3)]);
  assert.deepStrictEqual(plain(out.map(g => g.label)), ['acme/B', 'acme/y', 'Zeta/x']);
});

test('rows keep their order and the place given, as the same objects', () => {
  const rows = [row('acme/a', 1), row('acme/b', 2), row('acme/a', 3)];
  const out = boxGroups('new', rows, 'new');
  assert.strictEqual(out[0].rows.length, 2);
  assert.strictEqual(out[0].rows[0], rows[0]);
  assert.strictEqual(out[0].rows[1], rows[2]);
  assert.strictEqual(out[1].rows[0], rows[1]);
  for (const g of out) g.items.forEach((it, i) => { assert.strictEqual(it.place, 'new'); assert.strictEqual(it.row, g.rows[i]); });
});

test('names differing only in case share one heading, labelled with the first spelling', () => {
  const out = boxGroups('running', [row('Acme/A', 1), row('acme/a', 2)]);
  assert.strictEqual(out.length, 1);
  assert.strictEqual(out[0].label, 'Acme/A');
  assert.strictEqual(out[0].rows.length, 2);
});

test('box headings follow the owner order of the tree, owner before name', () => {
  const out = boxGroups('running', [row('a-b/y', 1), row('a/x', 2)]);
  assert.deepStrictEqual(plain(out.map(g => g.label)), ['a/x', 'a-b/y']);
});

test('a name with no slash is its own heading, sorted with the others', () => {
  const out = boxGroups('running', [row('local', 1), row('acme/a', 2), row('zeta/z', 3)]);
  assert.deepStrictEqual(plain(out.map(g => g.label)), ['acme/a', 'local', 'zeta/z']);
});

test('box keys are per box and apart from the tree keys', () => {
  const a = boxGroups('running', [row('acme/a', 1)]);
  const b = boxGroups('other', [row('acme/a', 1)]);
  assert.ok(a[0].key.startsWith('brepo:running/'));
  assert.ok(b[0].key.startsWith('brepo:other/'));
  assert.notStrictEqual(a[0].key, b[0].key);
  for (const g of [...a, ...b]) for (const p of ['repo:', 'owner:', 'org:']) assert.ok(!g.key.startsWith(p));
});

test('the state view puts each row in its box, in box order, and the boxes in repository headings', () => {
  const r = (nwo, id, st, cls) => ({ key: id, nwo, st, cls, s: {} });
  const rows = [r('b/x', 'r1', 'working'), r('a/y', 'o1', 'done'), r('a/z', 'n1', 'waiting', 'new'), r('a/y', 'r2', 'idle'), r('b/x', 'l1', 'waiting', 'later')];
  ctx.rows = rows;
  const tree = vm.runInContext('workTreeByState(rows)', ctx);
  assert.deepStrictEqual(plain(tree.map(b => b.key)), ['box:new', 'box:later', 'box:running', 'box:other']);
  const view = tree.map(b => b.items.map(g => [g.kind, g.label, g.items.map(i => `${i.place}:${i.row.key}`)]));
  assert.deepStrictEqual(plain(view), [
    [['brepo', 'a/z', ['new:n1']]],
    [['brepo', 'b/x', ['later:l1']]],
    [['brepo', 'a/y', ['box:r2']], ['brepo', 'b/x', ['box:r1']]],
    [['brepo', 'a/y', ['box:o1']]],
  ]);
  ctx.rows = [r('a/z', 'n1', 'waiting', 'new')];
  assert.deepStrictEqual(plain(vm.runInContext('workTreeByState(rows)', ctx).map(b => b.key)), ['box:new']);
});
