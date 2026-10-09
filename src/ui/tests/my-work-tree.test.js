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
const actions = fs.readFileSync(path.join(__dirname, '..', 'actions.js'), 'utf8');
const cutActions = re => {
  const m = actions.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const ctx = vm.createContext({
  boards: [],
  STATE_ORDER: { working: 0, waiting: 1, done: 2 }, stampSecs: () => 0,
  // What the page's other files give: a session's state is its own `state` here, and a board is served as it is named.
  sessionState: s => s.waiting ? 'waiting' : s.state, restingState: s => s.rest || 'stopped', workBoardOf: (repo, board) => board, answeredGates: new Map(), HUB_REF: 'hub:',
  workRepos: () => [], esc: x => String(x), workGlyphHtml: st => `<g ${st}>`,
});
vm.runInContext([
  cutActions(/^const hubWake = [^\n]*;/m),
  cutActions(/^const HUB_WAKE_TITLE = [^\n]*;/m),
  cutActions(/^function hubWakeBlocked[\s\S]*?^}/m),
  cutActions(/^function hubWakeWhy[\s\S]*?^}/m),
  cut(/^const WORK_BOXES = [\s\S]*?^\];/m),
  cut(/^const WORK_RUNNING = [^\n]*;/m),
  cut(/^const workBoxOf = [^\n]*;/m),
  cut(/^function workRowOrder[\s\S]*?^}/m),
  cut(/^const workStateOrder = [^\n]*;/m),
  cut(/^const WORKS_ON_PERSON = [^\n]*;/m),
  cut(/^const workOwnerName = [^\n]*;/m),
  cut(/^function workOwnerGroups[\s\S]*?^}/m),
  cut(/^const WORK_HUB_WORD = [\s\S]*?\};/m),
  cut(/^const workHubWord = [^\n]*;/m),
  cut(/^function workRepoHubChips[\s\S]*?^}/m),
  cut(/^function workPlaceHubChips[\s\S]*?^}/m),
  cut(/^const workChipsOnly = [^\n]*;/m),
  cut(/^function workHiddenHubWaits[\s\S]*?^}/m),
  cut(/^function workListedRepos[\s\S]*?^}/m),
  cut(/^function workListedView[\s\S]*?^}/m),
  cut(/^function workTreeByParent[\s\S]*?^}/m),
  cut(/^function workHeadHtml[\s\S]*?^}/m),
  cut(/^function workSummaryHtml[\s\S]*?^}/m),
  cut(/^function workHubWakeHtml[\s\S]*?^}/m),
  cut(/^function workHubChipsHtml[\s\S]*?^}/m),
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
  const rows = [r('b/x', 'r1', 'working'), r('a/y', 'o1', 'done'), r('a/z', 'n1', 'waiting', 'new'), r('a/y', 'r2', 'idle'), r('b/x', 'l1', 'waiting')];
  ctx.rows = rows;
  const tree = vm.runInContext('workTreeByState(rows)', ctx);
  assert.deepStrictEqual(plain(tree.map(b => b.key)), ['box:new', 'box:running', 'box:other']);
  const view = tree.map(b => b.items.map(g => [g.kind, g.label, g.items.map(i => `${i.place}:${i.row.key}`)]));
  assert.deepStrictEqual(plain(view), [
    [['brepo', 'a/z', ['new:n1']]],
    [['brepo', 'a/y', ['box:r2']], ['brepo', 'b/x', ['box:r1']]],
    [['brepo', 'a/y', ['box:o1']], ['brepo', 'b/x', ['box:l1']]],
  ]);
  ctx.rows = [r('a/z', 'n1', 'waiting', 'new')];
  assert.deepStrictEqual(plain(vm.runInContext('workTreeByState(rows)', ctx).map(b => b.key)), ['box:new']);
});

/* ── hubs as chips on the headings (#599) ── */
const run = (code, vars) => { Object.assign(ctx, vars); return vm.runInContext(code, ctx); };
const hubRow = (id, state, extra = {}) => ({ board: `b-${id}`, session: { id, kind: 'hub', state, present: state !== 'stopped', ...extra }, task: null });
const hubInfo = (id, parent, key = id) => ({ id, parent, key, slug: `b-${id}` });
const judgedOf = obj => new Map(Object.entries(obj));
const chipsOf = (repo, judged = new Map()) => plain(run('workRepoHubChips(repo, 1, judged)', { repo, judged }));

test('the state words: failure and unknown never read as waiting, and a stopped hub is not running', () => {
  const w = st => run('workHubWord(st)', { st });
  assert.strictEqual(w('working'), '作業中');
  assert.strictEqual(w('restarting'), '作業中');
  for (const st of ['done', 'idle', 'waiting', 'permission']) assert.strictEqual(w(st), '待機中');
  assert.strictEqual(w('failed'), 'エラー');
  assert.strictEqual(w('unknown'), '状態不明');
  assert.strictEqual(w('something-new'), '状態不明');
  for (const st of ['stopped', 'ended', 'none']) assert.strictEqual(w(st), '動いていない');
});

test('chips: the repository own hub first, then parent-task hubs by key; a stopped hub still has one', () => {
  const repo = { nwo: 'acme/w', hubs: [hubInfo('p2', true, '#9'), hubInfo('own', false), hubInfo('p1', true, '#2')],
    rows: [hubRow('own', 'stopped')], hubSessions: [{ board: 'b-p2', session: hubRow('p2', 'working').session }, { board: 'b-p1', session: hubRow('p1', 'idle').session }] };
  const chips = chipsOf(repo);
  assert.deepStrictEqual(chips.map(c => c.hub.id), ['own', 'p1', 'p2']);
  assert.strictEqual(chips[0].word, '動いていない');
  assert.strictEqual(chips[0].key, 'b-own/hub:own');
  assert.strictEqual(chips[0].isHub, true);
});

test('chips: a hub with no session has none, and an ended parent-task hub only while it waits', () => {
  const repo = { nwo: 'acme/w', hubs: [hubInfo('own', false), hubInfo('p1', true), hubInfo('p2', true)], rows: [hubRow('own', 'done')],
    hubSessions: [{ board: 'b-p1', session: hubRow('p1', 'ended').session }, { board: 'b-p2', session: hubRow('p2', 'ended').session }] };
  assert.deepStrictEqual(chipsOf(repo).map(c => c.hub.id), ['own']);
  const waits = judgedOf({ 'b-p2/hub:p2': { live: [{ kind: 'gate' }], cls: 'new' } });
  assert.deepStrictEqual(chipsOf(repo, waits).map(c => [c.hub.id, c.waits, c.cls]), [['own', 0, null], ['p2', 1, 'new']]);
  assert.strictEqual(chipsOf({ nwo: 'acme/w', hubs: [hubInfo('own', false)], rows: [], hubSessions: [] }).length, 0);
});

test('chips: what waits comes from the judged entry of the hub; an answered gate is not waited on', () => {
  const waiting = hubRow('own', 'waiting', { waiting: { slug: 'b-own', id: 'g1' } });
  const repo = { nwo: 'acme/w', hubs: [hubInfo('own', false)], rows: [waiting], hubSessions: [] };
  const judged = judgedOf({ 'b-own/hub:own': { live: [{ kind: 'gate' }, { kind: 'gate' }], cls: null } });
  const c = chipsOf(repo, judged)[0];
  assert.deepStrictEqual([c.waits, c.cls], [2, null]);
  const gates = ctx.answeredGates;
  try {
    ctx.answeredGates = new Map([['b-own/g1', true]]);
    // The judged entry is the page's own reading of the same answered gates; the row itself is no longer waiting.
    assert.strictEqual(plain(run('workRepoHubChips(repo, 1, new Map())', { repo }))[0].s.waiting, null);
  } finally {
    ctx.answeredGates = gates;
  }
});

const chip = (id, parent, key = id, slug = `b-${id}`, waits = 0) => ({ id, key: `${slug}/hub:${id}`, nwo: 'acme/w', hub: { id, parent, key: parent ? key : id, slug }, s: { present: true }, st: 'idle', waits, word: '待機中' });
const parentNode = (key, hub, extra = {}) => ({ key: `parent:${key}`, kind: 'parent', parent: { key, hub, ...extra }, rows: [], items: [], hubs: [] });

test('placing: a parent-task hub goes to the parent it runs; the repo own hub and a missing parent get none', () => {
  const own = chip('own', false), p1 = chip('p1', true, '#2', 'b-p1');
  const nodes = [parentNode('#2', 'b-p1'), parentNode('#3', 'b-own'), parentNode('#4', 'b-p1', { missing: true })];
  const rest = plain(run('workPlaceHubChips(chips, nodes)', { chips: [own, p1], nodes }));
  assert.deepStrictEqual(plain(nodes.map(n => n.hubs.map(c => c.id))), [['p1'], [], []]);
  assert.deepStrictEqual(rest.map(c => c.id), ['own']);
});

test('placing: a hub no parent heading takes is left over; one chip is never placed twice', () => {
  const p1 = chip('p1', true, '#2', 'b-p1');
  let nodes = [parentNode('#7', 'b-other')];
  assert.deepStrictEqual(plain(run('workPlaceHubChips(chips, nodes)', { chips: [p1], nodes })).map(c => c.id), ['p1']);
  nodes = [parentNode('#2', 'b-p1'), parentNode('#5', 'b-p1')];
  const rest = plain(run('workPlaceHubChips(chips, nodes)', { chips: [p1], nodes }));
  assert.strictEqual(nodes.flatMap(n => n.hubs).length, 1);
  assert.strictEqual(nodes[0].hubs.length, 1);
  assert.strictEqual(rest.length, 0);
});

const repoA = { nwo: 'acme/a', carrier: 'b-own', parents: [{ key: '#2', hub: 'b-p1', number: 2 }] };
const trow = (id, parent) => ({ key: id, id, repo: repoA, nwo: 'acme/a', st: 'idle', s: {}, task: { id, parent } });
const treeOf = (rows, chipsByRepo, repos = [repoA]) => plain(run('workTreeByParent(rows, [], byRepo, repos)', { rows, byRepo: chipsByRepo, repos }));
const reposOf = tree => tree.flatMap(o => o.items);

test('the parent tree has no hub rows; the hub is a chip on its heading', () => {
  const hub = { key: 'h', id: 'b-own/hub:own', repo: repoA, nwo: 'acme/a', st: 'idle', s: {}, isHub: true, task: null };
  const own = chip('own', false), p1 = chip('p1', true, '#2', 'b-p1');
  const tree = run('workTreeByParent(rows, [], byRepo, repos)', { rows: [hub, trow('t1', '#2'), trow('t2')], byRepo: new Map([[repoA, [own, p1]]]), repos: [repoA] });
  const [repoNode] = tree[0].items;
  assert.strictEqual(repoNode.rows.length, 2);
  assert.ok(!repoNode.rows.includes(hub));
  assert.deepStrictEqual(plain(repoNode.hubs.map(c => c.id)), ['own']);
  assert.strictEqual(repoNode.hub.id, 'own');
  const [parent, none] = repoNode.items;
  assert.deepStrictEqual(plain(parent.hubs.map(c => c.id)), ['p1']);
  assert.deepStrictEqual(plain(parent.items.map(i => i.row.key)), ['t1']);
  assert.deepStrictEqual(plain(none.items.map(i => i.row.key)), ['t2']);
});

test('the parent tree places a turn row (a task with nothing running) under its parent, or under none', () => {
  const turn = id => ({ ...trow(id, id === 't1' ? '#2' : undefined), turn: true, st: 'done' });
  const tree = treeOf([turn('t1'), turn('t2')], new Map());
  const [parent, none] = reposOf(tree)[0].items;
  assert.deepStrictEqual(parent.items.map(i => i.row.key), ['t1']);
  assert.deepStrictEqual(none.items.map(i => i.row.key), ['t2']);
});

test('the parent tree: a repository with only a chip has a node, one with neither has none', () => {
  const only = treeOf([], new Map([[repoA, [chip('own', false)]]]));
  assert.strictEqual(reposOf(only).length, 1);
  assert.deepStrictEqual(reposOf(only)[0].items, []);
  assert.strictEqual(reposOf(only)[0].hubs.length, 1);
  assert.deepStrictEqual(treeOf([], new Map()), []);
});

test('hidden waits: the hubs below a heading, not its own', () => {
  const node = { hubs: [chip('x', false, 'x', 'b-x', 9)], items: [
    { hubs: [chip('p1', true, '#2', 'b-p1', 2)], items: [{ row: {} }] },
    { hubs: [], items: [{ hubs: [chip('p2', true, '#3', 'b-p2', 3)], items: [] }] },
    { row: {} },
  ] };
  assert.strictEqual(run('workHiddenHubWaits(node)', { node }), 5);
  assert.strictEqual(run('workHiddenHubWaits(node)', { node: { hubs: [chip('x', false, 'x', 'b-x', 9)], items: [] } }), 0);
});

test('the folded summary says the hub waits only when some do', () => {
  assert.ok(!run('workSummaryHtml([], 0)', {}).includes('account_tree'));
  const html = run('workSummaryHtml([], 3)', {});
  assert.ok(html.includes('account_tree') && html.includes('hub があなたを待っています 3 件'));
});

test('the chip is its own button with the state word, and the waiting badge only when something waits', () => {
  const quiet = run('workHubChipsHtml([c], "acme/a", true)', { c: chip('own', false) });
  assert.match(quiet, /<button type="button" class="wk-hub" data-wk-hub="b-own\/hub:own"/);
  assert.ok(quiet.includes('待機中') && !quiet.includes('front_hand'));
  const busy = run('workHubChipsHtml([c], "acme/a", true)', { c: { ...chip('p1', true, '#2', 'b-p1', 2), st: 'stopped', word: '動いていない', cls: 'new' } });
  assert.ok(busy.includes('hub #2') && busy.includes('front_hand') && busy.includes('wk-hub-wait new') && busy.includes('hub を起動'));
  assert.strictEqual(run('workHubChipsHtml([], "x")', {}), '');
});

const srow = (nwo, id, st, cls, isHub) => ({ key: id, nwo, st, cls, isHub, s: {} });
const stateTree = (rows, chips) => plain(run('workTreeByState(rows, chips)', { rows, chips }));
const schip = (nwo, id, present) => ({ ...chip(id, false), nwo, s: { present } });

test('the state view: a hub row is dropped from 実行中 and そのほか but stays in 新着', () => {
  const tree = stateTree([srow('a/y', 'h1', 'idle', null, true), srow('a/y', 'h2', 'waiting', 'new', true), srow('a/y', 't1', 'working')], []);
  assert.deepStrictEqual(tree.map(b => b.key), ['box:new', 'box:running']);
  assert.deepStrictEqual(tree.map(b => b.rows.map(r => r.key)), [['h2'], ['t1']]);
});

test('the state view: a running hub is a chip on its repository heading in 実行中, a stopped one in そのほか', () => {
  const tree = stateTree([], [schip('a/y', 'on', true), schip('b/z', 'off', false)]);
  assert.deepStrictEqual(tree.map(b => b.key), ['box:running', 'box:other']);
  const [running, other] = tree.map(b => b.items);
  assert.deepStrictEqual([running[0].kind, running[0].label, running[0].rows, running[0].hubs.map(c => c.id)], ['brepo', 'a/y', [], ['on']]);
  assert.deepStrictEqual([other[0].label, other[0].hubs.map(c => c.id)], ['b/z', ['off']]);
});

test('box headings keep their order with chips, and a chip joins the heading of its rows', () => {
  const out = plain(run('workBoxRepoGroups("running", rows, "box", chips)', { rows: [row('b/x', 1)], chips: [schip('a/y', 'on', true), schip('b/x', 'on2', true)] }));
  assert.deepStrictEqual(out.map(g => g.label), ['a/y', 'b/x']);
  assert.deepStrictEqual(out.map(g => [g.rows.length, g.hubs.length]), [[0, 1], [1, 1]]);
});

test('the chip names the hub by its key only for a parent-task hub on a repository heading', () => {
  const html = (c, named) => run('workHubChipsHtml([c], "acme/a", named)', { c, named });
  const own = html({ ...chip('own', false), hub: { id: 'own', parent: false, key: null, slug: 'b-own' } }, true);
  assert.ok(!own.includes('undefined') && !own.includes('null'));
  assert.ok(own.includes('<span>hub</span>'));
  const onRepo = html(chip('p1', true, '#2', 'b-p1'), true);
  assert.ok(onRepo.includes('<span>hub #2</span>'));
  const onParent = html(chip('p1', true, '#2', 'b-p1'), false);
  assert.ok(onParent.includes('<span>hub</span>') && !onParent.includes('hub #2'));
  const noKey = html({ ...chip('p1', true, '#2', 'b-p1'), hub: { id: 'p1', parent: true, key: null, slug: 'b-p1' } }, true);
  assert.ok(noKey.includes('<span>hub</span>') && !noKey.includes('null'));
});

test('the start hint is only on a stopped hub, not an ended one', () => {
  const hint = st => run('workHubChipsHtml([c], "x", false)', { c: { ...chip('own', false), st } }).includes('hub を起動');
  assert.ok(hint('stopped') && hint('none'));
  assert.ok(!hint('ended') && !hint('working'));
});

test('chips: a hub session the hubs list does not name still gets a chip, as the server falls back to the session', () => {
  const stray = hubRow('x', 'idle', { key: '#8' });
  const repo = { nwo: 'acme/w', hubs: [], rows: [hubRow('own', 'idle')], hubSessions: [{ board: 'b-x', session: stray.session }] };
  const chips = chipsOf(repo);
  assert.deepStrictEqual(chips.map(c => [c.hub.id, c.hub.parent, c.hub.key, c.hub.slug]), [['own', false, null, 'b-own'], ['x', true, '#8', 'b-x']]);
});

test('placing: two parents sharing a hub slug each get the chip of their own key', () => {
  const second = chip('p2', true, '#5', 'b-shared');
  const nodes = [parentNode('#2', 'b-shared'), parentNode('#5', 'b-shared')];
  const rest = plain(run('workPlaceHubChips(chips, nodes)', { chips: [second], nodes }));
  assert.deepStrictEqual(plain(nodes.map(n => n.hubs.map(c => c.id))), [[], ['p2']]);
  assert.strictEqual(rest.length, 0);
});

test('a heading with only chips has nothing to fold; a node with items or an empty note keeps its fold button', () => {
  // The pieces the heading calls are stubbed for this test only, and put back after.
  const had = ['workFoldButton', 'workHubChipsHtml', 'workSegments', 'nav'].map(k => [k, k in ctx, ctx[k]]);
  const html = node => run('workHeadHtml(node, false)', { node, nav: {}, workSegments: () => [], workFoldButton: () => '<fold>', workHubChipsHtml: () => '<chips>' });
  try {
    const only = html({ kind: 'repo', nwo: 'a/b', key: 'repo:a/b', rows: [], items: [], hubs: [{}] });
    assert.ok(only.includes('wk-fold-gap') && !only.includes('<fold>'));
    assert.ok(html({ kind: 'repo', nwo: 'a/b', key: 'k', rows: [], items: [{}], hubs: [{}] }).includes('<fold>'));
    assert.ok(html({ kind: 'band', label: 'x', key: 'band:new', rows: [], items: [], hubs: undefined, empty: 'e' }).includes('<fold>'));
  } finally {
    for (const [k, had_, v] of had) { if (had_) ctx[k] = v; else delete ctx[k]; }
  }
});

test('the folded hub-waits item is an image with its own label', () => {
  const html = run('workSummaryHtml([], 2)', {});
  assert.ok(html.includes('role="img" aria-label="hub があなたを待っています 2 件"'));
});

test('a hub that is not running reads as not running even with a gate open, and keeps its waiting badge', () => {
  const stopped = hubRow('own', 'idle', { present: false, waiting: { slug: 'b-own', id: 'g9' } });
  const repo = { nwo: 'acme/w', hubs: [hubInfo('own', false)], rows: [stopped], hubSessions: [] };
  const judged = judgedOf({ 'b-own/hub:own': { live: [{ kind: 'gate' }], cls: 'new' } });
  const [c] = run('workRepoHubChips(repo, 1, judged)', { repo, judged });
  assert.deepStrictEqual([c.st, c.word, c.waits], ['stopped', '動いていない', 1]);
  const html = run('workHubChipsHtml([c], "acme/a", false)', { c });
  assert.ok(html.includes('wk-hub off') && html.includes('hub を起動') && html.includes('front_hand'));
});

test('placing: a keyed chip no parent matches goes to the repository heading, even when a parent shares its slug', () => {
  const lone = chip('p9', true, '#9', 'b-shared');
  const nodes = [parentNode('#2', 'b-shared')];
  const rest = plain(run('workPlaceHubChips(chips, nodes)', { chips: [lone], nodes }));
  assert.deepStrictEqual(rest.map(c => c.id), ['p9']);
  assert.strictEqual(nodes[0].hubs.length, 0);
});

test('placing: a chip with no key falls back to the parent whose hub has its slug', () => {
  const keyless = { ...chip('p1', true, '#2', 'b-p1'), hub: { id: 'p1', parent: true, key: null, slug: 'b-p1' } };
  const nodes = [parentNode('#2', 'b-p1')];
  assert.strictEqual(plain(run('workPlaceHubChips(chips, nodes)', { chips: [keyless], nodes })).length, 0);
  assert.deepStrictEqual(plain(nodes[0].hubs.map(c => c.id)), ['p1']);
});

// workListedRepos: an idle repository is left out of 「いまの仕事」.
const lrA = { nwo: 'acme/a' }, lrB = { nwo: 'acme/b' }, lrC = { nwo: 'zed/c' };
const lrow = (repo, st, cls = null, extra = {}) => ({ key: `${repo.nwo}#${st}${cls}`, id: `${repo.nwo}#${st}${cls}`, repo, nwo: repo.nwo, st, cls, s: {}, ...extra });
const lchip = (repo, present, waits = 0, parent = false) => ({ ...chip(parent ? 'p1' : 'own', parent, '#2', parent ? 'b-p1' : 'b-own', waits), repo, nwo: repo.nwo, s: { present } });
const listedOf = (repos, rows, turns, byRepo, keep) => plain(run('[...workListedRepos(repos, rows, turns, byRepo, keep)].map(r => r.nwo)', { repos, rows, turns, byRepo: new Map(byRepo), keep: keep ?? null }));

test('listed repositories: one with only a stopped own hub is left out', () => {
  assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [lchip(lrA, false)]]]), []);
});

test('listed repositories: rows that are done, stopped or failed with no class leave it out', () => {
  const rows = ['done', 'stopped', 'failed'].map(st => lrow(lrA, st));
  assert.deepStrictEqual(listedOf([lrA], rows, [], []), []);
});

test('listed repositories: a running hub keeps it, whatever its state word says', () => {
  for (const st of ['idle', 'done', 'failed']) {
    assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [{ ...lchip(lrA, true), st }]]]), ['acme/a']);
  }
});

test('listed repositories: a stopped hub with something waiting on the person keeps it', () => {
  assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [lchip(lrA, false, 1)]]]), ['acme/a']);
});

test('listed repositories: a session that is running keeps it', () => {
  for (const st of ['working', 'idle', 'unknown', 'restarting']) assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, st)], [], []), ['acme/a'], st);
});

test('listed repositories: a read done row with nothing else lets it idle out; one with a PR or a park keeps it (#600)', () => {
  const live = (...kinds) => ({ live: kinds.map(kind => ({ kind })) });
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'done', null, live())], [], []), []);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'done', null, live('done'))], [], []), []);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'failed', null, live('failed'))], [], []), []);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'done', null, live('pr'))], [], []), ['acme/a']);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'done', null, live('parked'))], [], []), ['acme/a']);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'done', 'new', live('done'))], [], []), ['acme/a']);
});

test('listed repositories: a waiting or permission row keeps it even with no class', () => {
  for (const st of ['waiting', 'permission']) assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, st)], [], []), ['acme/a'], st);
});

test('listed repositories: a row with no session, which is in the lists by its class, keeps it', () => {
  const turn = lrow(lrA, 'waiting', 'new', { s: {}, turn: true });
  assert.deepStrictEqual(listedOf([lrA], [], [turn], []), ['acme/a']);
  assert.deepStrictEqual(listedOf([lrA], [], [lrow(lrA, 'done', null, { turn: true })], []), []);
});

test('listed repositories: one that could not be read stays, so its notice is not lost', () => {
  assert.deepStrictEqual(listedOf([{ ...lrA, error: 'boom' }], [], [], []), ['acme/a']);
});

test('listed repositories: the one holding the selection stays, though it is idle', () => {
  assert.deepStrictEqual(listedOf([lrA, lrB], [], [], [], lrB), ['acme/b']);
});

test('listed repositories: a running parent-task hub alone keeps its repository', () => {
  assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [lchip(lrA, true, 0, true)]]]), ['acme/a']);
});

test('listed repositories: idle ones are left out of both groupings, and an owner with only idle ones has no node', () => {
  const idle = lrow(lrB, 'done'), busy = lrow(lrA, 'working');
  const byRepo = new Map([[lrA, [lchip(lrA, true)]], [lrB, [lchip(lrB, false)]], [lrC, [lchip(lrC, false)]]]);
  const v = run('workListedView([a, b, c], rows, [], byRepo)', { a: lrA, b: lrB, c: lrC, rows: [busy, idle], byRepo });
  assert.deepStrictEqual(plain(v.repos.map(r => r.nwo)), ['acme/a']);
  const state = stateTree(v.rows, v.chips);
  assert.deepStrictEqual(state.map(b => b.key), ['box:running']);
  assert.ok(!JSON.stringify(state).includes('acme/b') && !JSON.stringify(state).includes('zed/c'));
  assert.deepStrictEqual(stateTree(run('workListedView([a, b], rows, [], new Map())', { a: lrA, b: lrB, rows: [idle] }).rows, []), []);
  const tree = plain(run('workTreeByParent(v.rows, [], v.chipsByRepo, v.repos)', { v }));
  assert.deepStrictEqual(tree.map(o => o.key), ['owner:acme']);
  assert.deepStrictEqual(tree[0].items.map(r => r.nwo), ['acme/a']);
});

test('listed repositories: the keep given to the view keeps an idle repository with its rows and chip', () => {
  const idle = lrow(lrB, 'done');
  const byRepo = new Map([[lrB, [lchip(lrB, false)]]]);
  const v = run('workListedView([b], rows, [], byRepo, b)', { b: lrB, rows: [idle], byRepo });
  assert.deepStrictEqual([v.rows.length, v.chips.length, v.repos.length], [1, 1, 1]);
});

/* ── the wake button beside a hub's chip ── */
const wakeChip = (hub = {}, extra = {}) => ({ ...chip('own', false), hub: { id: 'own', parent: false, key: 'own', slug: 'b-own', unseen: 2, state: { present: true }, ...hub }, wakeBase: '/b/b-own', ...extra });
const chipHtml = c => run('workHubChipsHtml([c], "acme/a", true)', { c });

test('wake: no button when nothing is unseen or the count is missing', () => {
  for (const unseen of [0, undefined]) assert.ok(!chipHtml(wakeChip({ unseen })).includes('data-wk-wake'));
});

test('wake: an enabled button is the chip\'s sibling with the count, never inside the chip', () => {
  const html = chipHtml(wakeChip());
  assert.match(html, /<button type="button" class="wk-hub-wake" data-wk-wake="b-own\/hub:own" aria-label="hub を起こす（受信箱の未確認 2 件）"/);
  assert.ok(!/<button type="button" class="wk-hub-wake"[^>]* disabled/.test(html));
  const chipPart = html.slice(html.indexOf('<button type="button" class="wk-hub"'), html.indexOf('</button>') + 9);
  assert.ok(!chipPart.includes('data-wk-wake') && !chipPart.includes('notifications_active'));
  assert.ok(html.indexOf('data-wk-wake') > html.indexOf('</button>'));
});

test('wake: disabled with the reason when the hub is stopped, its board is not here, or a wake is on its way', () => {
  const off = c => { const m = chipHtml(c).match(/<button type="button" class="wk-hub-wake"[^>]*>/)[0]; return [/ disabled/.test(m), m.match(/title="([^"]*)"/)[1]]; };
  assert.deepStrictEqual(off(wakeChip({ state: { present: false } })), [true, 'hub が止まっています']);
  assert.deepStrictEqual(off(wakeChip({}, { wakeBase: null })), [true, 'この hub のボードはこのサーバーにありません']);
  run('hubWake.busy["b-own"] = true', {});
  // The label names the button whatever the reason; the reason is the title.
  assert.ok(chipHtml(wakeChip({ state: { present: false } })).includes('aria-label="hub を起こす（受信箱の未確認 2 件）"'));
  try {
    assert.deepStrictEqual(off(wakeChip()), [true, '起こしています']);
  } finally {
    run('delete hubWake.busy["b-own"]', {});
  }
});

test('wake: the reason it did not wake is escaped, in a live region, and gone once nothing is unseen', () => {
  const plainEsc = ctx.esc;
  ctx.esc = x => String(x).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  run('hubWake.why["b-own"] = "入力しませんでした: <b>x</b>"', {});
  try {
    assert.ok(chipHtml(wakeChip()).includes('<span class="hub-wake-why wk-hub-why" role="status">入力しませんでした: &lt;b&gt;x&lt;/b&gt;</span>'));
    assert.ok(chipHtml(wakeChip({ unseen: 0 })).indexOf('hub-wake-why') < 0);
    assert.strictEqual(run('hubWake.why["b-own"]', {}), undefined);
  } finally {
    ctx.esc = plainEsc;
    run('delete hubWake.why["b-own"]', {});
  }
});

test('wake: a hub with no unseen messages forgets its reason', () => {
  run('hubWake.why["b-own"] = "入力しませんでした"', {});
  assert.strictEqual(run('hubWakeWhy(h)', { h: { slug: 'b-own', unseen: 1 } }), '入力しませんでした');
  assert.strictEqual(run('hubWakeWhy(h)', { h: { slug: 'b-own', unseen: 0 } }), '');
  assert.strictEqual(run('hubWake.why["b-own"]', {}), undefined);
});

test('wake: a chip carries the base of its hub\'s board only when this server has it', () => {
  const repo = { nwo: 'acme/w', hubs: [hubInfo('own', false)], rows: [hubRow('own', 'done')], hubSessions: [] };
  run('boards = [{ slug: "b-own" }]', {});
  try {
    assert.strictEqual(chipsOf(repo)[0].wakeBase, '/b/b-own');
    run('boards = [{ slug: "other" }]', {});
    assert.strictEqual(chipsOf(repo)[0].wakeBase, null);
  } finally {
    run('boards = []', {});
  }
});

test('wake: a repository heading and a parent heading both show the button of their hub', () => {
  const repoNode = { key: 'repo:acme/w', kind: 'repo', nwo: 'acme/w', label: 'acme/w', rows: [], items: [], hubs: [wakeChip()] };
  assert.ok(run('workHeadHtml(node, false)', { node: repoNode }).includes('data-wk-wake="b-own/hub:own"'));
  // The parent heading takes the same chips through `workHubChipsHtml`.
  const p1 = wakeChip({ id: 'p1', parent: true, key: '#2', slug: 'b-p1' }, { key: 'b-p1/hub:p1', wakeBase: '/b/b-p1' });
  assert.ok(run('workHubChipsHtml([c], "t")', { c: p1 }).includes('data-wk-wake="b-p1/hub:p1"'));
});
