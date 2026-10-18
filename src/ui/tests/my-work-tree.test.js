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
  sessionState: s => s.waiting && !s.went ? 'waiting' : s.state, gateFirst: s => !!s.waiting && !s.went, restingState: s => s.rest || 'stopped', workBoardOf: (repo, board) => board, answeredGates: new Map(), HUB_REF: 'hub:', PARENT_REF: 'parent:', nav: { task: null }, workCleanable: () => false, workCleanRowHtml: () => '',
  workRepos: () => [], SESS_REF: 'sess:', work: { doc: null }, esc: x => String(x), pad2: n => String(n).padStart(2, '0'), when: x => `W(${x})`, workGlyphHtml: st => `<g ${st}>`,
  // What a row's own markup reads from the page's other files.
  sessionLabel: () => ({ text: 'the title' }), sessionTip: () => '', agoLabel: () => '', minutesSince: () => 0, prRefNumber: () => null, agentStateOf: () => 'working',
  requestText: () => '', workAwayChip: () => '', workParkedHtml: () => '', workSaidHtml: m => `<said>${m}</said>`, kindOf: () => ['gate'], workGateWho: (t, passed) => passed ? 'who-passed' : 'who', WORK_SUBAGENTS_SHOWN: 4,
});
vm.runInContext([
  cutActions(/^const hubWake = [^\n]*;/m),
  cutActions(/^const HUB_WAKE_TITLE = [^\n]*;/m),
  cutActions(/^function hubWakeBlocked[\s\S]*?^}/m),
  cutActions(/^function hubWakeWhy[\s\S]*?^}/m),
  cutActions(/^const hubTitleRead = [^\n]*;/m),
  cutActions(/^const HUB_TITLE_READ_LABEL = [^\n]*;/m),
  cutActions(/^const hubTitleClock = [^\n]*;/m),
  cutActions(/^function hubTitleWords[\s\S]*?^}/m),
  cutActions(/^function hubTitleMarkHtml[\s\S]*?^}/m),
  cutActions(/^function hubTitleReadBlocked[\s\S]*?^}/m),
  cutActions(/^function hubTitleReadButtonHtml[\s\S]*?^}/m),
  cutActions(/^function hubTitleReadWhy[\s\S]*?^}/m),
  cutActions(/^function hubTitleWhyHtml[\s\S]*?^}/m),
  cut(/^const workHubBase = [^\n]*;/m),
  cut(/^function workTitleToolsHtml[\s\S]*?^}/m),
  cut(/^const WORK_RUNNING = [^\n]*;/m),
  cut(/^const workMerged = [^\n]*;/m),
  cut(/^const workInShown = [^\n]*;/m),
  cut(/^function workNewShown[\s\S]*?^}/m),
  cut(/^function workReadAll\(\)[\s\S]*?^}/m),
  cut(/^const WORK_LISTED = \[[^\]]*\];/m),
  cut(/^function workRows[\s\S]*?^}/m),
  cut(/^function workBetter[\s\S]*?^}/m),
  cut(/^function workShape[\s\S]*?^}/m),
  cut(/^function workNewOrder[\s\S]*?^}/m),
  cut(/^const WORK_PROGRESS = \{[\s\S]*?^\};/m),
  cut(/^function workSegments[\s\S]*?^}/m),
  cut(/^const workParentName = [^\n]*;/m),
  cut(/^function workParentMeta[\s\S]*?^}/m),
  cut(/^function workParentOpenHtml[\s\S]*?^}/m),
  cut(/^function workRequestsHtml[\s\S]*?^}/m),
  cut(/^function workHubStartButtonHtml[\s\S]*?^}/m),
  cut(/^function workRowOrder[\s\S]*?^}/m),
  cut(/^const workStateOrder = [^\n]*;/m),
  cut(/^const WORKS_ON_PERSON = [^\n]*;/m),
  cut(/^const WORK_HUB_WORD = [\s\S]*?\};/m),
  cut(/^const workHubWord = [^\n]*;/m),
  cut(/^function workRepoHubChips[\s\S]*?^}/m),
  cut(/^function workPlaceHubChips[\s\S]*?^}/m),
  cut(/^const workPercent = [^\n]*;/m),
  cut(/^const workInboxHtml = [\s\S]*?^\]\.filter\(Boolean\);/m),
  cut(/^function workRowHtml[\s\S]*?^}/m),
  cut(/^function workItemHtml[\s\S]*?^}/m),
  cut(/^function workRefocus[\s\S]*?^}/m),
  cut(/^function workHubWakeButtonHtml[\s\S]*?^}/m),
  cut(/^function workHubWhyHtml[\s\S]*?^}/m),
  cut(/^const workNewest = [^\n]*;/m),
  cut(/^function workNewOrder[\s\S]*?^}/m),
  cut(/^function workFoldButton[\s\S]*?^}/m),
  cut(/^function workHubGroupName[\s\S]*?^}/m),
  cut(/^function workHiddenHubWaits[\s\S]*?^}/m),
  cut(/^function workListedRepos[\s\S]*?^}/m),
  cut(/^function workListedView[\s\S]*?^}/m),
  cut(/^function workTreeByParent[\s\S]*?^}/m),
  cut(/^function workReadAllButtonHtml[\s\S]*?^}/m),
  cut(/^function workHeadHtml[\s\S]*?^}/m),
  cut(/^function workSummaryHtml[\s\S]*?^}/m),
  cut(/^function workHubWakeHtml[\s\S]*?^}/m),
  cut(/^function workHubChipsHtml[\s\S]*?^}/m),
].join('\n'), ctx);
const plain = v => JSON.parse(JSON.stringify(v));
ctx.input = null;
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
const treeOf = (rows, chipsByRepo, repos = [repoA]) => plain(run('workTreeByParent(rows, byRepo, repos)', { rows, byRepo: chipsByRepo, repos }));
const reposOf = tree => tree;

test('the parent tree: the own hub heads the group of the rows with no parent, first in the repository, and is not a task row', () => {
  const hub = { key: 'h', id: 'b-own/hub:own', repo: repoA, nwo: 'acme/a', st: 'idle', s: {}, isHub: true, task: null };
  const own = chip('own', false), p1 = chip('p1', true, '#2', 'b-p1');
  const [repoNode] = run('workTreeByParent(rows, byRepo, repos)', { rows: [hub, trow('t1', '#2'), trow('t2')], byRepo: new Map([[repoA, [own, p1]]]), repos: [repoA] });
  assert.strictEqual(repoNode.rows.length, 2);
  assert.ok(!repoNode.rows.includes(hub));
  assert.deepStrictEqual(plain(repoNode.hubs.map(c => c.id)), ['own']);
  const [none, parent] = repoNode.items;
  assert.deepStrictEqual([none.key, none.kind, none.hub.id, none.head.id], ['none:acme/a', 'hub', 'own', 'own']);
  assert.deepStrictEqual(plain(none.items.map(i => i.row.key)), ['t2']);
  assert.strictEqual(none.head, own);
  assert.deepStrictEqual(plain(none.rows.map(r => r.key)), ['t2']);
  assert.deepStrictEqual(plain(parent.hubs.map(c => c.id)), ['p1']);
  assert.deepStrictEqual(plain(parent.items.map(i => i.row.key)), ['t1']);
  assert.deepStrictEqual(plain(parent.rows.map(r => r.key)), ['t1']);
  assert.strictEqual(parent.head, p1);
});

test('the parent tree: the hub heads its group, whatever order the tasks sort in', () => {
  const p1 = chip('p1', true, '#2', 'b-p1');
  const early = { ...trow('a-first', '#2'), st: 'waiting' };
  const late = trow('z-last', '#2');
  const [parent] = reposOf(treeOf([late, early], new Map([[repoA, [p1]]])))[0].items;
  assert.strictEqual(parent.head.id, 'p1');
  assert.deepStrictEqual(parent.items.map(i => i.row.key), ['a-first', 'z-last']);
});

test('the parent tree: a parent-task hub no parent group takes is a row after the own hub, named by its key', () => {
  const own = chip('own', false), stray = chip('p9', true, '#9', 'b-p9');
  const [none] = reposOf(treeOf([trow('t2')], new Map([[repoA, [stray, own]]])))[0].items;
  assert.deepStrictEqual(none.items.map(i => [i.row.id, !!i.named]), [['p9', true], ['t2', false]]);
  assert.strictEqual(none.head.id, 'own');
  assert.deepStrictEqual(none.hubs.map(c => c.id), ['own']);
});

test('the parent tree: the hub group is first, above the parents, and only the hub, or only loose rows, is enough for it', () => {
  const [none, parent] = reposOf(treeOf([trow('t1', '#2'), trow('t2')], new Map([[repoA, [chip('own', false)]]])))[0].items;
  assert.deepStrictEqual([none.kind, parent.kind], ['hub', 'parent']);
  const onlyHub = reposOf(treeOf([], new Map([[repoA, [chip('own', false)]]])))[0].items;
  assert.deepStrictEqual(onlyHub.map(n => [n.kind, n.rows.length, n.items.length]), [['hub', 0, 0]]);
  assert.strictEqual(onlyHub[0].head.id, 'own');
  const onlyLoose = reposOf(treeOf([trow('t2')], new Map()))[0].items;
  assert.deepStrictEqual(onlyLoose.map(n => [n.kind, n.hub, n.hubs.length, n.rows.length, n.items.length]), [['hub', null, 0, 1, 1]]);
  assert.strictEqual(onlyLoose[0].head, null);
  // Only tasks with a parent: no group of rows with no parent, and no own hub, so none at all.
  assert.deepStrictEqual(reposOf(treeOf([trow('t1', '#2')], new Map()))[0].items.map(n => n.kind), ['parent']);
});

test('the parent tree: the repository counts task rows only, and its chip is its own hub', () => {
  const own = chip('own', false, 'own', 'b-own', 3);
  const [repoNode] = reposOf(treeOf([trow('t1', '#2'), trow('t2')], new Map([[repoA, [own]]])));
  assert.strictEqual(repoNode.rows.length, 2);
  assert.deepStrictEqual(repoNode.hubs.map(c => c.id), ['own']);
});

test('the parent tree places a turn row (a task with nothing running) under its parent, or under none', () => {
  const turn = id => ({ ...trow(id, id === 't1' ? '#2' : undefined), turn: true, st: 'done' });
  const tree = treeOf([turn('t1'), turn('t2')], new Map());
  const [none, parent] = reposOf(tree)[0].items;
  assert.deepStrictEqual(parent.items.map(i => i.row.key), ['t1']);
  assert.deepStrictEqual(none.items.map(i => i.row.key), ['t2']);
});

test('the parent tree: a repository with only a hub has a node, one with neither has none', () => {
  const only = treeOf([], new Map([[repoA, [chip('own', false)]]]));
  assert.strictEqual(reposOf(only).length, 1);
  assert.strictEqual(reposOf(only)[0].hubs.length, 1);
  assert.deepStrictEqual(treeOf([], new Map()), []);
});

test('hidden waits: the hub rows below a heading, each once, and not the chip on the heading itself', () => {
  const hubRowOf = (id, waits, key = id) => ({ row: { ...chip(id, true, key, `b-${id}`, waits), isHub: true } });
  const shared = hubRowOf('p1', 2);
  const node = { hubs: [chip('x', false, 'x', 'b-x', 9)], items: [
    { hubs: [], items: [shared, { row: {} }] },
    { hubs: [], items: [{ hubs: [], items: [hubRowOf('p2', 3)] }, shared] },
    { row: { ...chip('x', false, 'x', 'b-x', 9), isHub: true } },
    { row: { key: 't', waits: 7 } },
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
const schip = (nwo, id, present) => ({ ...chip(id, false), nwo, s: { present } });

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

test('a heading shows the chips of its hubs only while it is folded, and always has its fold button', () => {
  // The pieces the heading calls are stubbed for this test only, and put back after.
  const had = ['workHubChipsHtml', 'workSegments', 'nav'].map(k => [k, k in ctx, ctx[k]]);
  const html = (node, folded) => run('workHeadHtml(node, folded)', { node, folded, nav: {}, workSegments: () => [], workHubChipsHtml: () => '<chips>' });
  try {
    for (const node of [{ kind: 'repo', nwo: 'a/b', key: 'repo:a/b', rows: [], items: [], hubs: [{}] }, { kind: 'brepo', label: 'a/b', key: 'k', rows: [], items: [{}], hubs: [{}] },
      { kind: 'parent', key: 'p', parent: { key: '#2', repo: {} }, rows: [], items: [], hubs: [{}] }, { kind: 'hub', key: 'none:a/b', hub: null, rows: [], items: [], hubs: [{}] }]) {
      assert.ok(html(node, true).includes('<chips>'), node.kind);
      assert.ok(!html(node, false).includes('<chips>'), node.kind);
      for (const folded of [true, false]) assert.ok(html(node, folded).includes('data-wk-fold'), node.kind);
    }
    assert.ok(!html({ kind: 'repo', nwo: 'a/b', key: 'repo:a/b', rows: [], items: [], hubs: [{}] }, false).includes('wk-fold-gap'));
  } finally {
    for (const [k, had_, v] of had) { if (had_) ctx[k] = v; else delete ctx[k]; }
  }
});

test('the group of the rows with no parent is named for its hub and what it is doing, with the unseen count only when above 0', () => {
  const head = node => run('workHeadHtml(node, false)', { node: { key: 'none:a/b', kind: 'hub', rows: [], items: [], hubs: [], ...node } });
  const own = (word, unseen) => ({ ...chip('own', false), word, hub: { id: 'own', parent: false, key: 'own', slug: 'b-own', unseen } });
  assert.ok(head({ hub: own('作業中', 0) }).includes('hub ・ 作業中') && !head({ hub: own('作業中', 0) }).includes('受信箱'));
  const unseen = head({ hub: own('待機中', 3) });
  assert.ok(unseen.includes('hub ・ 待機中') && unseen.includes('受信箱 未確認 3') && unseen.includes('role="img" aria-label="受信箱の未確認 3 件"'));
  assert.ok(head({ hub: own('動いていない', 0) }).includes('hub ・ 動いていない'));
  const none = head({ hub: null });
  assert.ok(none.includes('>hub<') && !none.includes('親なし') && !none.includes('・'));
});

test('a group the hub heads has the fold button and the count for its heading, and the summary only while folded', () => {
  const node = { key: 'none:a/b', kind: 'hub', rows: [{ st: 'working' }], items: [{}], hubs: [], head: chip('own', false) };
  const open = run('workHeadHtml(node, false)', { node });
  assert.ok(open.includes('data-wk-fold="none:a/b"') && open.includes('wk-count') && !open.includes('wk-sum') && !open.includes('wk-head-main'));
  assert.ok(run('workHeadHtml(node, true)', { node }).includes('wk-sum'));
  const parent = { ...node, kind: 'parent', parent: { key: '#2', number: 2, title: 'Parent' } };
  assert.ok(run('workHeadHtml(node, false)', { node: parent }).includes('#2 Parent'));
});

test('the group name escapes what it draws', () => {
  const plainEsc = ctx.esc;
  ctx.esc = x => String(x).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  try {
    const hub = { ...chip('own', false), word: '<b>x</b>', hub: { id: 'own', parent: false, key: 'own', slug: 'b-own', unseen: 1 } };
    const html = run('workHeadHtml(node, false)', { node: { key: 'none:a/b', kind: 'hub', rows: [], items: [], hubs: [hub], hub } });
    assert.ok(html.includes('hub ・ &lt;b&gt;x&lt;/b&gt;') && !html.includes('<b>x</b>'));
  } finally {
    ctx.esc = plainEsc;
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

test('listed repositories: a stopped hub with requests it has not read keeps it, so its row can say so and start it', () => {
  const hub = { ...lchip(lrA, false), hub: { ...lchip(lrA, false).hub, requestsUnseen: 2 } };
  assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [hub]]]), ['acme/a']);
  assert.deepStrictEqual(listedOf([lrA], [], [], [[lrA, [lchip(lrA, false)]]]), []);
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

test('listed repositories: idle ones are left out of the tree', () => {
  const idle = lrow(lrB, 'done'), busy = lrow(lrA, 'working');
  const byRepo = new Map([[lrA, [lchip(lrA, true)]], [lrB, [lchip(lrB, false)]], [lrC, [lchip(lrC, false)]]]);
  const v = run('workListedView([a, b, c], rows, [], byRepo)', { a: lrA, b: lrB, c: lrC, rows: [busy, idle], byRepo });
  assert.deepStrictEqual(plain(v.repos.map(r => r.nwo)), ['acme/a']);
  const tree = plain(run('workTreeByParent(v.rows, v.chipsByRepo, v.repos)', { v }));
  assert.deepStrictEqual(tree.map(r => r.nwo), ['acme/a']);
  assert.ok(!JSON.stringify(tree).includes('acme/b') && !JSON.stringify(tree).includes('zed/c'));
  assert.deepStrictEqual(plain(run('workTreeByParent(rows, new Map(), [a, b])', { a: lrA, b: lrB, rows: run('workListedView([a, b], rows, [], new Map())', { a: lrA, b: lrB, rows: [idle] }).rows })), []);
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
  assert.match(html, /<button type="button" class="wk-hub-wake" data-wk-wake="b-own\/hub:own" data-wake-slug="b-own" aria-label="hub を起こす（受信箱の未確認 2 件）"/);
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

test('wake: a folded repository heading and a folded parent heading both show the button of their hub', () => {
  const repoNode = { key: 'repo:acme/w', kind: 'repo', nwo: 'acme/w', label: 'acme/w', rows: [], items: [], hubs: [wakeChip()] };
  assert.ok(run('workHeadHtml(node, true)', { node: repoNode }).includes('data-wk-wake="b-own/hub:own"'));
  assert.ok(!run('workHeadHtml(node, false)', { node: repoNode }).includes('data-wk-wake'));
  // The parent heading takes the same chips through `workHubChipsHtml`.
  const p1 = wakeChip({ id: 'p1', parent: true, key: '#2', slug: 'b-p1' }, { key: 'b-p1/hub:p1', wakeBase: '/b/b-p1' });
  assert.ok(run('workHubChipsHtml([c], "t")', { c: p1 }).includes('data-wk-wake="b-p1/hub:p1"'));
});

/* ── a hub as a row (#610) ── */
const hubItem = (hub = {}, extra = {}) => ({ ...wakeChip(hub), id: 'b-own/hub:own', isHub: true, turn: false, task: null, data: { now: 1 },
  s: { present: true, agent: 'claude', agentSession: { model: 'opus', contextPercent: 42, lastMessage: 'did a thing\nmore', subagents: [{ type: 'x' }] } }, ...extra });
const itemHtml = (r, place = 'tree', named = false) => run('workItemHtml(r, place, named)', { r, place, named });
// The markup with every `<button ...>...</button>` taken out one level deep, to find what sits outside the row button.
const outsideRow = html => html.replace(/<button type="button" class="wk-row[\s\S]*?<\/button>/, '');

test('a hub row carries what a session row does, with the inbox counts in place of the branch', () => {
  const html = itemHtml(hubItem({ unseen: 2, seen: 4 }, {}), 'tree');
  const rowPart = html.match(/<button type="button" class="wk-row[\s\S]*?<\/button>/)[0];
  assert.ok(rowPart.includes('data-wk="b-own/hub:own"') && rowPart.includes('<g idle>'));
  assert.ok(rowPart.includes('claude · opus') && rowPart.includes('42%') && rowPart.includes('サブエージェント 1'));
  assert.ok(rowPart.includes('<said>did a thing\nmore</said>'));
  assert.ok(rowPart.includes('受信箱 未確認 2') && rowPart.includes('確認済み 4'));
  assert.ok(rowPart.includes('<span class="wk-tag">hub</span>'));
  const withBranch = itemHtml(hubItem({}, { s: { present: true, branch: 'feat/x', agentSession: {} } }), 'tree');
  assert.ok(!withBranch.includes('feat/x'));
});

test('a row whose worker went on past its gate keeps the gate line, marked, and is not in the waiting state', () => {
  const row = went => itemHtml({ ...hubItem(), isHub: false, id: 'k', key: 'k', st: went ? 'working' : 'waiting',
    s: { present: true, agent: 'claude', waiting: { kind: 'plan' }, went, agentSession: {} } }, 'tree');
  assert.ok(row(true).includes('who-passed') && row(true).includes('<g working>') && !row(true).includes('<g waiting>'));
  assert.ok(row(false).includes('who') && !row(false).includes('who-passed') && row(false).includes('<g waiting>'));
});

test('a hub row has no inbox spans for empty counts, and names a parent-task hub by its key only when told to', () => {
  const quiet = itemHtml(hubItem({ unseen: 0, seen: 0 }), 'tree');
  assert.ok(!quiet.includes('受信箱') && !quiet.includes('確認済み'));
  const parent = hubItem({ id: 'p1', parent: true, key: '#9', slug: 'b-p1', unseen: 0 }, { key: 'b-p1/hub:p1', id: 'b-p1/hub:p1' });
  assert.ok(itemHtml(parent, 'tree', true).includes('<span class="wk-tag">hub #9</span>'));
  assert.ok(itemHtml(parent, 'tree', false).includes('<span class="wk-tag">hub</span>') && !itemHtml(parent, 'tree', false).includes('#9'));
  assert.ok(!itemHtml(hubItem({ unseen: 0 }), 'tree', true).includes('undefined'));
});

test('a hub row has its wake button beside the row button, never inside it', () => {
  const html = itemHtml(hubItem({ unseen: 2 }), 'tree');
  assert.match(html, /^<div class="wk-band-row wk-hub-row" data-wk-cell="tree\/b-own\/hub:own"><button type="button" class="wk-row/);
  assert.ok(!html.match(/<button type="button" class="wk-row[\s\S]*?<\/button>/)[0].includes('data-wk-wake'));
  const rest = outsideRow(html);
  assert.ok(rest.includes('<span class="wk-acts">') && rest.includes('data-wk-wake="b-own/hub:own"') && !rest.includes('data-wk-read'));
  // Nothing unseen: no wake button, and no empty actions either.
  const quiet = itemHtml(hubItem({ unseen: 0 }), 'tree');
  assert.ok(!quiet.includes('data-wk-wake') && !quiet.includes('wk-acts'));
});

test('a parent-task hub row says where its title stands in the title line, and has the read button beside the row', () => {
  const hub = { parent: true, key: 'ALPHA-9', titleState: { status: 'read', reading: false, total: 5, completed: 2 } };
  const html = itemHtml(hubItem(hub), 'tree');
  const rowPart = html.match(/<button type="button" class="wk-row[\s\S]*?<\/button>/)[0];
  assert.ok(rowPart.includes('サブ Issue 完了 2 / 5') && !rowPart.includes('data-hub-title-read'));
  const rest = outsideRow(html);
  assert.ok(rest.includes('data-hub-title-read="b-own"') && rest.includes('hub-title-why wk-hub-why'));
  // The repository's own hub has no title state: nothing is added.
  const own = itemHtml(hubItem({ parent: false }), 'tree');
  assert.ok(!own.includes('hub-title') && !own.includes('data-hub-title-read'));
});

test('a parent heading draws the title state of the hub that gave it its title, found by slug, beside its main button', () => {
  const repo = { hubs: [
    { slug: 'x', titleState: { status: 'pending', reading: true } },
    { slug: 'b-own', titleState: { status: 'failed', why: 'nope', retryAt: 0 } },
  ] };
  const head = parent => run('workHeadHtml(node, false)', { node: { kind: 'parent', key: 'p', parent: { key: 'acme/widget#9', repo, ...parent }, rows: [], items: [], hubs: [] },
    nav: {}, workSegments: () => [] });
  ctx.boards = [{ slug: 'b-own' }];
  const html = head({ titleHub: 'b-own' });
  assert.ok(html.includes('data-hub-title-read="b-own"') && html.includes('タイトルを読めませんでした: nope'));
  assert.ok(!html.includes('data-hub-title-read="x"') && !html.includes('タイトルを読み込み中'));
  // Outside the heading's main button, never inside it.
  assert.ok(!html.match(/<(button|span) class="wk-head-main"[\s\S]*?<\/(button|span)>/)[0].includes('hub-title'));
  assert.ok(!head({}).includes('hub-title'));
  // A hub that is read has its progress in the button's tooltip only: the heading counts merges itself.
  repo.hubs[1].titleState = { status: 'read', reading: false, total: 5, completed: 2 };
  const read = head({ titleHub: 'b-own' });
  assert.ok(!read.includes('hub-title-mark') && read.includes('サブ Issue 完了 2 / 5'));
  ctx.boards = [];
});

test('a hub row in 新着 has 既読 as well as the wake button, both beside the row', () => {
  const rest = outsideRow(itemHtml(hubItem({ unseen: 1 }, { cls: 'new' }), 'tree'));
  assert.ok(rest.includes('data-wk-read="b-own/hub:own"') && rest.includes('data-wk-wake="b-own/hub:own"'));
  assert.ok(!outsideRow(itemHtml(hubItem({ unseen: 1 }), 'tree')).includes('data-wk-read'));
});

test('a task row is a bare row button unless it is new, and then has 既読 beside it, wherever it sits', () => {
  const task = { key: 'b/t1', id: 'b/t1', st: 'working', s: { present: true }, data: { now: 1 }, task: { id: 't1' } };
  assert.ok(itemHtml(task, 'tree').startsWith('<button type="button" class="wk-row'));
  assert.ok(itemHtml({ ...task, cls: 'new' }, 'tree').startsWith('<div class="wk-band-row" '));
});

test('a folded node counts the hubs below it once, and a stopped hub is not counted from its row twice', () => {
  const hub = { ...hubItem(), waits: 2 };
  const node = { hubs: [], items: [{ items: [{ row: hub, place: 'tree' }] }, { items: [{ row: hub, place: 'new' }] }] };
  assert.strictEqual(run('workHiddenHubWaits(node)', { node }), 2);
});

test('the new rows are the ones of class new, in the order the tree draws them: a group\'s head, then what is under it', () => {
  const mk = (id, cls) => ({ key: id, id, cls, st: 'waiting', s: {} });
  const tree = [{ key: 'repo:a', kind: 'repo', items: [
    { key: 'g1', kind: 'hub', head: mk('h1', 'new'), items: [{ row: mk('t1', 'new'), place: 'tree' }, { row: mk('t2', null), place: 'tree' }] },
    { key: 'g2', kind: 'parent', head: mk('h2', null), items: [{ row: mk('t3', 'new'), place: 'tree' }] },
    { key: 'g3', kind: 'hub', head: null, items: [{ row: mk('t4', 'new'), place: 'tree' }] },
  ] }];
  assert.deepStrictEqual(plain(run('workNewOrder(tree)', { tree })), ['h1', 't1', 't3', 't4']);
});

test('a parent-task hub no parent group takes is tagged with its key in the own group', () => {
  const stray = { ...chip('p1', true, '#9', 'b-p1'), nwo: 'acme/a' };
  const [repoNode] = treeOf([], new Map([[repoA, [stray]]]));
  const [none] = repoNode.items;
  assert.strictEqual(none.items[0].named, true);
  assert.ok(run('workItemHtml(r, "tree", n)', { r: { ...hubItem({ id: 'p1', parent: true, key: '#9', slug: 'b-p1', unseen: 0 }), key: none.items[0].row.key }, n: none.items[0].named }).includes('hub #9'));
});

test('a hub row puts the wake reason on a line of its own, outside the buttons and the row', () => {
  run('hubWake.why["b-own"] = "入力しませんでした: 長い理由"', {});
  try {
    const html = itemHtml(hubItem({ unseen: 2 }), 'tree');
    const rest = outsideRow(html);
    const acts = rest.match(/<span class="wk-acts">[\s\S]*?<\/button><\/span>/)[0];
    assert.ok(acts.includes('data-wk-wake') && !acts.includes('hub-wake-why'));
    assert.ok(rest.includes('<span class="hub-wake-why wk-hub-why" role="status">入力しませんでした: 長い理由</span></div>'));
    assert.ok(!html.match(/<button type="button" class="wk-row[\s\S]*?<\/button>/)[0].includes('hub-wake-why'));
    // A folded heading's chip keeps the reason beside its button.
    assert.ok(chipHtml(wakeChip()).includes('hub-wake-why'));
  } finally {
    run('delete hubWake.why["b-own"]', {});
  }
});

test('a hub chip carries the entry and the away count of its judged entry, as a session row does', () => {
  const repo = { nwo: 'acme/w', hubs: [hubInfo('own', false)], rows: [hubRow('own', 'done')], hubSessions: [] };
  const entry = { id: 'b-own/hub:own' };
  const [c] = chipsOf(repo, judgedOf({ 'b-own/hub:own': { entry, live: [], cls: null, away: 3 } }));
  assert.deepStrictEqual([c.entry, c.away], [entry, 3]);
  const [none] = chipsOf(repo);
  assert.deepStrictEqual([none.entry, none.away], [null, 0]);
  ctx.workAwayChip = r => r > 0 ? `+${r}` : '';
  try {
    assert.ok(itemHtml({ ...hubItem(), away: 3 }, 'tree').includes('<span>+3</span>'));
  } finally { ctx.workAwayChip = () => ''; }
});

test('focus after a rebuild goes back to the copy in the same cell, else to the first', () => {
  const log = [];
  const el = (name, cell, held) => ({ dataset: { wkCell: cell }, matches: sel => sel === held, querySelector: sel => sel === held ? inner : null, focus: () => log.push(name) });
  const inner = { focus: () => log.push('inner') };
  const band = el('band', 'new/k', '[data-wk="k"]'), group = el('group', 'tree/k', '[data-wk="k"]');
  const first = { focus: () => log.push('first') };
  const root = { querySelectorAll: () => [band, group], querySelector: () => first };
  const go = (held, cell) => { log.length = 0; run('workRefocus(root, held, cell)', { root, held, cell }); return log.slice(); };
  assert.deepStrictEqual(go('[data-wk="k"]', 'tree/k'), ['group']);
  assert.deepStrictEqual(go('[data-wk="k"]', 'new/k'), ['band']);
  assert.deepStrictEqual(go('[data-wk-wake="k"]', 'tree/k'), ['first']);
  assert.deepStrictEqual(go('[data-wk="k"]', 'gone'), ['first']);
  assert.deepStrictEqual(go('[data-wk="k"]', null), ['first']);
  assert.deepStrictEqual(go(null, 'tree/k'), []);
  // A wake button inside the wrapper of the cell is found within it.
  const wrap = { dataset: { wkCell: 'tree/k' }, matches: () => false, querySelector: () => inner };
  assert.deepStrictEqual((log.length = 0, run('workRefocus(root, held, cell)', { root: { querySelectorAll: () => [band, wrap], querySelector: () => first }, held: '[data-wk-wake="k"]', cell: 'tree/k' }), log.slice()), ['inner']);
});

/* ── the list as the organisation's repositories, hubs and sessions (#728) ── */

test('with one repository chosen the tree is that repository\'s groups, with no repository node', () => {
  const rows = [trow('t1', '#2'), trow('t2')];
  const byRepo = new Map([[repoA, [chip('own', false)]]]);
  const withHeadings = treeOf(rows, byRepo);
  assert.deepStrictEqual(withHeadings.map(n => n.kind), ['repo']);
  const flat = plain(run('workTreeByParent(rows, byRepo, [repoA], { headings: false })', { rows, byRepo, repoA }));
  assert.deepStrictEqual(flat.map(n => n.kind), ['hub', 'parent']);
  assert.deepStrictEqual(flat.map(n => n.key), withHeadings[0].items.map(n => n.key));
});

test('a parent group headed by its hub row says the parent, and how far it has got, in that row', () => {
  const p1 = chip('p1', true, '#2', 'b-p1');
  const parent = { key: '#2', number: 2, title: 'Ship it', merged: 1, total: 3, children: [{ id: 'a', progress: 'merged' }], stacked: true };
  const html = run('workItemHtml(r, "tree", false, p)', { r: { ...hubItem({ id: 'p1', parent: true, key: '#2', slug: 'b-p1', unseen: 0 }), hub: p1.hub }, p: parent });
  assert.ok(html.includes('#2 Ship it') && html.includes('1 / 3 マージ') && html.includes('wk-stack') && html.includes('class="wk-bar"'));
  // Without a hub the group keeps the parent heading, with its button to the parent.
  const [repoNode] = treeOf([trow('t1', '#2')], new Map());
  const [group] = repoNode.items;
  assert.deepStrictEqual([group.kind, group.head], ['parent', null]);
  assert.ok(run('workHeadHtml(node, false)', { node: group }).includes('data-wk-parent="#2"'));
});

test('a merged task is listed after its worker has gone, and keeps its repository; one not merged is not', () => {
  const doc = { now: 1, repos: [{ nwo: 'acme/a', hubs: [], rows: [
    { board: 'b', session: { id: 's1', kind: 'worker', state: 'ended' }, task: { id: 't1', prState: 'merged' } },
    { board: 'b', session: { id: 's2', kind: 'worker', state: 'ended' }, task: { id: 't2', prState: 'open' } },
    { board: 'b', session: { id: 's3', kind: 'worker', state: 'none', branchPr: { state: 'merged' } }, task: null },
  ] }] };
  assert.deepStrictEqual(plain(run('workRows(doc).map(r => r.s.id)', { doc })), ['s1', 's3']);
  const merged = { ...lrow(lrA, 'ended'), task: { id: 't1', prState: 'merged' } };
  assert.deepStrictEqual(listedOf([lrA], [merged], [], []), ['acme/a']);
  assert.deepStrictEqual(listedOf([lrA], [lrow(lrA, 'ended')], [], []), []);
});

test('a hub says how many requests it has not read, with the newest one\'s subject', () => {
  const hub = (extra, st = 'stopped', more = {}) => ({ ...hubItem({ unseen: 1, ...extra }), st, ...more });
  assert.strictEqual(run('workRequestsHtml(c)', { c: hub({ requestsUnseen: 0 }) }), '');
  assert.strictEqual(run('workRequestsHtml(c)', { c: hub({}) }), '');
  const inbox = [{ kind: 'report', seen: false, counted: true, subject: 'a report' }, { kind: 'request', seen: false, counted: false, subject: 'the hub\'s own' }, { kind: 'request', seen: false, counted: true, subject: 'newest <b>' }, { kind: 'request', seen: false, counted: true, subject: 'older' }, { kind: 'request', seen: true, subject: 'read' }];
  const line = run('workRequestsHtml(c)', { c: hub({ requestsUnseen: 2, inbox }) });
  assert.ok(line.includes('未読の依頼 2') && line.includes('「newest <b>」') && !line.includes('older') && !line.includes('a report') && !line.includes('own'));
  // The listed inbox keeps only the newest few, so the count can be more than it names.
  const far = run('workRequestsHtml(c)', { c: hub({ requestsUnseen: 3, inbox: [{ kind: 'report', seen: false, subject: 's' }] }) });
  assert.ok(far.includes('未読の依頼 3') && !far.includes('「'));
  assert.ok(run('workRowHtml(r)', { r: hub({ requestsUnseen: 2, inbox }) }).includes('未読の依頼 2'));
});

test('「hub を起動」 is beside a hub that is not running and has unread requests, and says why when it cannot', () => {
  const hub = (st, extra = {}, more = {}) => ({ ...hubItem({ unseen: 0, requestsUnseen: 1, ...extra }), st, ...more });
  const html = c => run('workHubStartButtonHtml(c)', { c });
  const ok = html(hub('stopped', {}, { wakeBase: '/b/own' }));
  assert.ok(ok.includes('data-wk-start="b-own/hub:own"') && ok.includes('hub を起動') && !ok.includes('disabled'));
  assert.ok(html(hub('none', {}, { wakeBase: '/b/own' })).includes('data-wk-start'));
  for (const st of ['working', 'idle', 'ended']) assert.strictEqual(html(hub(st, {}, { wakeBase: '/b/own' })), '', st);
  assert.strictEqual(html(hub('stopped', { requestsUnseen: 0 }, { wakeBase: '/b/own' })), '');
  const away = html(hub('stopped', {}, { wakeBase: null }));
  assert.ok(away.includes('disabled') && away.includes('このサーバーにありません'));
  const noKey = html(hub('stopped', { parent: true, key: null }, { wakeBase: '/b/p' }));
  assert.ok(noKey.includes('disabled') && noKey.includes('キーが分からない'));
  // A sibling of the row button, in the buttons beside it, never inside it.
  assert.ok(outsideRow(itemHtml(hub('stopped', {}, { wakeBase: '/b/own' }), 'tree')).includes('data-wk-start'));
});

test('a poll that only changes words does not change the shape, and a hub that heads a group is part of it', () => {
  const mk = (cls, head) => [{ key: 'g', kind: 'hub', head, hubs: head ? [head] : [], rows: [], items: [{ row: { key: 't1', cls }, place: 'tree' }] }];
  const h = { key: 'h1', cls: null };
  const shape = tree => JSON.stringify(run('workShape(tree, new Set())', { tree }));
  assert.strictEqual(shape(mk(null, h)), shape(mk('new', { ...h, cls: 'new' })));
  assert.notStrictEqual(shape(mk(null, h)), shape(mk(null, null)));
  assert.notStrictEqual(shape(mk(null, h)), shape(mk(null, { key: 'h2' })));
});

test('the new rows inside a folded group are not in the order 「処理したら次へ」 walks, but the group\'s head is', () => {
  const mk = (id, cls) => ({ key: id, id, cls, st: 'waiting', s: {} });
  const tree = [{ key: 'repo:a', kind: 'repo', items: [
    { key: 'g1', kind: 'hub', head: mk('h1', 'new'), items: [{ row: mk('t1', 'new'), place: 'tree' }] },
    { key: 'g2', kind: 'hub', head: mk('h2', null), items: [{ row: mk('t2', 'new'), place: 'tree' }] },
  ] }];
  const order = folded => plain(run('workNewOrder(tree, folded)', { tree, folded: new Set(folded) }));
  assert.deepStrictEqual(order([]), ['h1', 't1', 't2']);
  assert.deepStrictEqual(order(['g1']), ['h1', 't2']);
  assert.deepStrictEqual(order(['repo:a']), []);
});

test('a parent\'s hub head has a button of its own to the parent\'s overview, beside the row and not inside it', () => {
  const node = { key: 'parent:a/#2', kind: 'parent', parent: { key: '#2', number: 2, title: 'Ship it' }, rows: [], items: [], hubs: [], head: chip('p1', true, '#2', 'b-p1') };
  const head = run('workHeadHtml(node, false)', { node });
  assert.ok(head.includes('data-wk-parent="#2"') && head.includes('aria-label="親タスクの概要（#2 Ship it）"'));
  assert.ok(!head.includes('wk-row'));
  const current = run('workHeadHtml(node, false)', { node: { ...node, parent: { ...node.parent } }, });
  assert.ok(!current.includes('aria-current'));
  const none = (extra, kind = 'parent') => run('workHeadHtml(node, false)', { node: { ...node, kind, parent: { ...node.parent, ...extra } } });
  assert.ok(!none({ missing: true }).includes('data-wk-parent'));
  assert.ok(!none({}, 'hub').includes('data-wk-parent'));
});

test('a task on a parent-task hub\'s board is grouped under the parent that hub runs, with or without a parent of its own', () => {
  const repo = { ...repoA, hubs: [{ id: 'own', parent: false, key: 'own', slug: 'b-own' }, { id: 'p1', parent: true, key: '#2', slug: 'b-p1' }, { id: 'p9', parent: true, key: '#9', slug: 'b-p9' }],
    parents: [{ key: '#2', hub: 'b-p1', number: 2 }] };
  const onHub = { ...trow('asked'), repo, hubSlug: 'b-p1' };
  const onOwn = { ...trow('plain'), repo, hubSlug: 'b-own' };
  const named = { ...trow('named', '#2'), repo, hubSlug: 'b-own' };
  // A parent-task hub whose parent the document does not list leaves the task to what it names.
  const orphan = { ...trow('orphan'), repo, hubSlug: 'b-p9' };
  const [repoNode] = plain(run('workTreeByParent(rows, new Map(), [repo])', { rows: [onHub, onOwn, named, orphan], repo }));
  const keys = repoNode.items.map(n => [n.key, n.rows.map(r => r.key)]);
  assert.deepStrictEqual(keys, [['none:acme/a', ['orphan', 'plain']], ['parent:acme/a/#2', ['asked', 'named']]]);
});

test('rows of a group keep to the order their tasks were made in, whatever their state or restarts; sessions with no task come last, by key', () => {
  const mk = (key, st, id, startedAt) => ({ key, st, s: { startedAt }, task: id ? { id } : null });
  const rows = [mk('d', 'working', null), mk('c', 'waiting', '20261018T000300Z-c', 1), mk('a', 'working', '20261018T000200Z-a', 99), mk('b', 'permission', '20261018T000100Z-b'), mk('e', 'done', null)];
  const sorted = plain(run('rows.sort(workRowOrder)', { rows }));
  assert.deepStrictEqual(sorted.map(r => r.key), ['b', 'a', 'c', 'd', 'e']);
});

test('what the list as shown counts and reads is the selected organisation\'s (and the repository chosen), not every organisation\'s', () => {
  const entry = (id, nwo) => [id, { cls: 'new', entry: { id, nwo } }];
  const judged = new Map([entry('a1', 'acme/x'), entry('a2', 'acme/y'), entry('b1', 'beta/z'), ['d', { cls: null, entry: { id: 'd', nwo: 'acme/x' } }]]);
  const writes = [];
  const vars = { work: { doc: {} }, workJudge: () => judged, workServerNow: () => 5, workWriteMarks: (nwo, patches) => writes.push([nwo, patches.map(p => p[0])]),
    orgScope: () => ({ current: 'acme', known: new Set(['acme', 'beta']) }), orgInScope: (sc, nwo) => nwo.split('/')[0] === sc.current, workRepoChoice: () => '' };
  Object.assign(ctx, vars);
  assert.strictEqual(run('workNewShown()'), 2);
  run('workReadAll()');
  assert.deepStrictEqual(plain(writes), [['acme/x', ['a1']], ['acme/y', ['a2']]]);
  // With one repository chosen, only its rows.
  ctx.workRepoChoice = () => 'acme/y';
  assert.strictEqual(run('workNewShown()'), 1);
  // Another organisation's new rows alone leave nothing to read in this list.
  ctx.orgScope = () => ({ current: 'zeta', known: new Set(['acme', 'beta', 'zeta']) });
  assert.strictEqual(run('workNewShown()'), 0);
});
