/* The owner level of 「いまの仕事」 (src/ui/my-work.js), run with `node --test`. my-work.js draws into the page as it
   loads, so only the pure helpers are cut out of the source and evaluated here. */
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
const ctx = vm.createContext({});
vm.runInContext([
  cut(/^const workOwnerName = [^\n]*;/m),
  cut(/^function workOwnerGroups[\s\S]*?^}/m),
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
