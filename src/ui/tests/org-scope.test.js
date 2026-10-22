/* What is scoped to the selected organisation outside the work list: the rail count, the board in the address, and the boards
   「すべて」 reads. The functions are cut out of the sources and run against stubs, as the other UI tests do. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cutFrom = (file, re) => { const m = read(file).match(re); assert.ok(m, `${file}: ${re}`); return m[0]; };
const plain = v => JSON.parse(JSON.stringify(v));
const scopeOf = (current, known) => () => ({ current, known: new Set(known) });
// The page's own `orgOf` and `orgInScope` (org.js), so the tests follow its rule rather than a copy of it.
const orgHelpers = cutFrom('org.js', /^const orgOf = [^\n]*;/m) + '\n' + cutFrom('org.js', /^const orgInScope = [^\n]*;/m);

function run(code, vars = {}) {
  const ctx = vm.createContext({ ...vars });
  vm.runInContext(orgHelpers + '\n' + code, ctx);
  return ctx;
}

test('workNewCount counts only the selected organisation\'s new rows, and an unlisted organisation\'s row still counts', () => {
  const row = (id, nwo, cls = 'new') => [id, { cls, entry: { id, nwo } }];
  const judged = new Map([row('a1', 'acme/x'), row('a2', 'acme/y'), row('b1', 'beta/z'), row('u1', 'ghost/q'), row('d', 'acme/x', null)]);
  const ctx = run(cutFrom('my-work.js', /^function workNewCount[\s\S]*?^}/m), {
    work: { doc: {} }, workJudge: () => judged, orgScope: scopeOf('acme', ['acme', 'beta']),
  });
  assert.strictEqual(vm.runInContext('workNewCount()', ctx), 3);
  ctx.orgScope = scopeOf('beta', ['acme', 'beta']);
  assert.strictEqual(vm.runInContext('workNewCount()', ctx), 2);
  ctx.work = { doc: null };
  assert.strictEqual(vm.runInContext('workNewCount()', ctx), 0);
});

test('readBoards keeps the selected organisation\'s boards and still drops finished ones', () => {
  const boards = [
    { slug: 'a', nwo: 'acme/a' }, { slug: 'a2', nwo: 'acme/a', finished: true },
    { slug: 'b', nwo: 'beta/b' }, { slug: 'g', nwo: 'ghost/g' },
  ];
  const ctx = run(cutFrom('core.js', /^const readBoards = [^\n]*;/m), { boards, orgScope: scopeOf('acme', ['acme', 'beta']) });
  assert.deepStrictEqual(plain(vm.runInContext('readBoards()', ctx)).map(b => b.slug), ['a', 'g']);
  ctx.orgScope = scopeOf('beta', ['acme', 'beta']);
  assert.deepStrictEqual(plain(vm.runInContext('readBoards()', ctx)).map(b => b.slug), ['b', 'g']);
});

test('boardFollowOrg: follows a board of another organisation once, and leaves a later choice alone', () => {
  const sets = [];
  const ctx = run(cutFrom('actions.js', /^let boardFollowedAddr = [^\n]*;/m) + cutFrom('actions.js', /^function boardFollowOrg[\s\S]*?^}/m), {
    multiBoard: true, nav: { view: 'agent', board: 'b' },
    boards: [{ slug: 'a', nwo: 'acme/a' }, { slug: 'b', nwo: 'beta/b' }],
    cur: 'acme', currentOrg: () => ctx.cur, setCurrentOrg: l => { sets.push(l); ctx.cur = l; },
  });
  const follow = () => vm.runInContext('boardFollowOrg()', ctx);
  follow();
  assert.deepStrictEqual(sets, ['beta']);
  // The person picks another organisation while the address stays: not undone.
  ctx.cur = 'acme';
  follow();
  assert.deepStrictEqual(sets, ['beta']);
  // The address moves to a board of the other organisation again.
  ctx.nav.board = 'a';
  follow();
  ctx.nav.board = 'b';
  follow();
  assert.deepStrictEqual(sets, ['beta', 'beta']);
});

test('boardFollowOrg: nothing for 「すべて」, nothing recorded while the board is unknown, and the work view forgets the address', () => {
  const sets = [];
  const ctx = run(cutFrom('actions.js', /^let boardFollowedAddr = [^\n]*;/m) + cutFrom('actions.js', /^function boardFollowOrg[\s\S]*?^}/m), {
    multiBoard: true, nav: { view: 'agent', board: 'all' }, boards: [],
    currentOrg: () => 'acme', setCurrentOrg: l => sets.push(l),
  });
  const follow = () => vm.runInContext('boardFollowOrg()', ctx);
  follow();
  assert.deepStrictEqual(sets, []);
  // The list is not in yet: looked at again once it is.
  ctx.nav.board = 'b';
  follow();
  assert.deepStrictEqual(sets, []);
  // Still the 'all' recorded before: the unknown board is not.
  assert.strictEqual(vm.runInContext('boardFollowedAddr', ctx), 'all');
  ctx.boards = [{ slug: 'b', nwo: 'beta/b' }];
  follow();
  assert.deepStrictEqual(sets, ['beta']);
  // The work view resets, so coming back to the same address is looked at again.
  ctx.nav.view = 'work';
  follow();
  assert.strictEqual(vm.runInContext('boardFollowedAddr', ctx), null);
  ctx.nav.view = 'agent';
  follow();
  assert.deepStrictEqual(sets, ['beta', 'beta']);
  // A board served alone has no organisations to follow.
  ctx.multiBoard = false;
  ctx.nav.board = 'zzz';
  follow();
  assert.deepStrictEqual(sets, ['beta', 'beta']);
});

test('othersNewCount counts only the selected organisation\'s new review records, as the rail\'s レビュー count is scoped', () => {
  const records = [{ id: 'a', repo: 'acme/x' }, { id: 'b', repo: 'beta/y' }, { id: 'b2', repo: 'beta/z' }, { id: 'old', repo: 'beta/y' }];
  const ctx = run(cutFrom('my-work-others-view.js', /^const othersNewCount = \(\) => \{[\s\S]*?^};/m), {
    others: { doc: {} }, othersRecords: () => records, othersMarks: () => ({}),
    othersNewIds: (rs, marks) => rs.filter(r => r.id !== 'old').map(r => r.id),
    orgScope: scopeOf('beta', ['acme', 'beta']),
  });
  // The page's own todoInScope (todo-view.js), which the rail's レビュー count reads too.
  vm.runInContext(cutFrom('todo-view.js', /^const todoInScope = [^\n]*;/m), ctx);
  assert.strictEqual(vm.runInContext('othersNewCount()', ctx), 2);
  ctx.orgScope = scopeOf('acme', ['acme', 'beta']);
  assert.strictEqual(vm.runInContext('othersNewCount()', ctx), 1);
  ctx.others = { doc: null };
  assert.strictEqual(vm.runInContext('othersNewCount()', ctx), 0);
});
