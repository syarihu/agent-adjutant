/* Unit tests for reading a parent-task hub's state beside the page's (issue #821).
   Run with `node --test src/ui/tests/panel-other.test.js`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};

function createContext(extra = {}) {
  const calls = [];
  const ctx = {
    assert,
    calls,
    BASE: '',
    navEpoch: 0,
    TOKEN: 'tok',
    answeredGates: new Map(),
    gateKey: g => g._slug ? `${g._slug}/${g.id}` : g.id,
    renderTaskPanel: () => { calls.push(['renderTaskPanel']); },
    taskDetailFollow: () => { calls.push(['taskDetailFollow']); },
    note: (msg, isErr) => { calls.push(['note', msg, isErr]); },
    boardApi: async (base, path) => {
      calls.push(['boardApi', base, path]);
      return { tasks: [], gates: [], waits: [] };
    },
    openTaskPanel: (id, pane) => { calls.push(['openTaskPanel', id, pane]); },
    openTaskDetail: (t, el) => {
      calls.push(['openTaskDetail', t, el]);
      ctx.taskDetail = { key: `detail:${ctx.nav?.board}/${t.id}`, board: ctx.nav?.board, slug: t._slug || '', id: t.id, gone: false };
    },
    onBoard: (slug, fn) => { calls.push(['onBoard', slug]); return fn && fn(); },
    humanColOf: t => t.humanCol || null,
    dialogStack: { isOpen: () => true },
    location: { pathname: '/b/repo1/', search: '' },
    lastStateJson: '',
    lastMinute: null,
    seenGateIds: null,
    resetViews: () => {},
    boardJob: null,
    allRound: false,
    emptyState: () => ({ tasks: [], gates: [] }),
    scopeAll: () => false,
    readBoards: () => [],
    mergeStates: () => ({ tasks: [], gates: [] }),
    boardStates: {},
    nav: { board: 'repo1', view: 'agent', task: null },
    ...extra,
  };
  if (ctx.nav && !ctx.nav.view) ctx.nav.view = 'agent';
  vm.createContext(ctx);

  const utilSrc = read('util.js');
  const coreSrc = read('core.js');
  const navSrc = read('nav.js');
  const cardsSrc = read('cards.js');
  const tdSrc = read('task-detail-view.js');

  vm.runInContext([
    // viewData, stateNow, withViewState
    cut(utilSrc, /^let viewData = null;[\s\S]*?\n\}/m).replace('let viewData =', 'var viewData =').replace('const stateNow =', 'var stateNow ='),
    // ref prefixes and checkers
    'const WORK_GATE_REF = "gate:";',
    cut(coreSrc, /^const HUB_REF = 'hub:';[\s\S]*?const isTaskRef = [^\n]*;/m),
    // nav helpers
    cut(navSrc, /^const hubTaskOf = [^\n]*;/m),
    cut(navSrc, /^let panelOwnerPick = [^\n]*;/m).replace('let panelOwnerPick =', 'var panelOwnerPick ='),
    cut(navSrc, /^function panelOwnerSlug\(ref\) \{[\s\S]*?\n\}/m),
    cut(navSrc, /^const panelOther = \{[\s\S]*?\nlet panelOtherBusy = false;\nlet panelOtherFailed = false;/m)
      .replace('const panelOther =', 'var panelOther =')
      .replace('let panelOtherBusy =', 'var panelOtherBusy =')
      .replace('let panelOtherFailed =', 'var panelOtherFailed ='),
    cut(navSrc, /^function panelOtherSlug\(\) \{[\s\S]*?\n\}/m),
    cut(navSrc, /^const panelSlug = [^\n]*;/m).replace('const panelSlug =', 'var panelSlug ='),
    cut(navSrc, /^async function refreshPanelOther[\s\S]*?\n\}/m),
    cut(navSrc, /^let panelStateMemo = [^\n]*;\n[\s\S]*?function panelState\(\) \{[\s\S]*?\n\}/m).replace('let panelStateMemo =', 'var panelStateMemo ='),
    cut(navSrc, /^function withPanelState\(fn\) \{[\s\S]*?\n\}/m),
    cut(navSrc, /^const scopeAll = [^\n]*;/m),
    cut(navSrc, /^function switchBoard\(\) \{[\s\S]*?\n\}/m),
    // taskDetailOwnerSlug from task-detail-view.js
    cut(tdSrc, /^let taskDetail = null;/m).replace('let taskDetail =', 'var taskDetail ='),
    cut(tdSrc, /^function taskDetailOwnerSlug\(\) \{[\s\S]*?\n\}/m),
    // openCardTask from cards.js
    cut(cardsSrc, /^function openCardTask\(task, el\) \{[\s\S]*?\n\}/m),
    // Bind all to ctx/globalThis
    'Object.assign(globalThis, { stateNow, withViewState, hubTaskOf, panelOwnerPick, panelOwnerSlug, panelOther, panelOtherSlug, panelSlug, refreshPanelOther, panelState, withPanelState, openCardTask, taskDetailOwnerSlug, switchBoard });',
  ].join('\n'), ctx);

  return ctx;
}

test('panelOwnerSlug resolves parent-task hub owner when task is not in state.tasks', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 't1' },
    state: {
      tasks: [{ id: 'local1' }],
      hubTasks: [
        { id: 't1', ownerHub: { slug: 'hub1' } },
        { id: 't2', ownerHub: { slug: 'repo1' } },
      ],
    },
  });

  // Task belongs to hub1
  assert.strictEqual(ctx.panelOwnerSlug('t1'), 'hub1');
  // Task belongs to page's own board
  assert.strictEqual(ctx.panelOwnerSlug('t2'), null);
  // Task is local to page's state.tasks (page task wins)
  assert.strictEqual(ctx.panelOwnerSlug('local1'), null);
  // Non-task refs
  assert.strictEqual(ctx.panelOwnerSlug('hub:1'), null);
  assert.strictEqual(ctx.panelOwnerSlug('session:s1'), null);
  // Not on resident server
  ctx.multiBoard = false;
  assert.strictEqual(ctx.panelOwnerSlug('t1'), null);
  // All view
  ctx.multiBoard = true;
  ctx.nav.board = 'all';
  assert.strictEqual(ctx.panelOwnerSlug('t1'), null);
});

test('panelOtherSlug falls back to taskDetailOwnerSlug when nav.task has no owner', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: null },
    state: {
      tasks: [],
      hubTasks: [{ id: 't1', ownerHub: { slug: 'hub1' } }],
    },
  });

  assert.strictEqual(ctx.panelOtherSlug(), null);

  ctx.taskDetail = { slug: 'hub1', gone: false };
  assert.strictEqual(ctx.panelOtherSlug(), 'hub1');

  // Closed dialog
  ctx.dialogStack.isOpen = () => false;
  assert.strictEqual(ctx.panelOtherSlug(), null);
});

test('panelSlug returns owner slug or falls back to nav.board', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 't1' },
    state: {
      tasks: [],
      hubTasks: [{ id: 't1', ownerHub: { slug: 'hub1' } }],
    },
  });

  assert.strictEqual(ctx.panelSlug(), 'hub1');
  ctx.nav.task = 'local1';
  assert.strictEqual(ctx.panelSlug(), 'repo1');
});

test('panelState returns state itself when panelOtherSlug is null', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: null },
    state: { tasks: [{ id: 'local1' }], gates: [], waits: [], hubTasks: [] },
  });

  assert.strictEqual(ctx.panelState(), ctx.state);
});

test('panelState keeps the page\'s own tasks and adds the owner hub\'s, tagged, and memoises the result', () => {
  const localState = {
    tasks: [{ id: 'local1' }],
    gates: [{ id: 'lg1' }],
    waits: [],
    hubTasks: [{ id: 'ht1', ownerHub: { slug: 'hub1' } }, { id: 'hx', ownerHub: { slug: 'hub2' } }],
  };
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'ht1' },
    state: localState,
  });
  const byId = (s, id) => s.tasks.find(t => t.id === id);

  // 1. Before the hub's state arrives: the page's own tasks and gates, and the hub cards tagged with their own board.
  const s1 = ctx.panelState();
  assert.notStrictEqual(s1, localState);
  assert.strictEqual(byId(s1, 'local1')._slug, undefined);
  assert.strictEqual(byId(s1, 'ht1')._slug, 'hub1');
  assert.strictEqual(byId(s1, 'ht1')._base, '/b/hub1');
  assert.strictEqual(byId(s1, 'hx')._slug, 'hub2');
  // Arrays made in the vm context have its prototype, so their length is compared.
  assert.strictEqual(s1.gates.length, 1);
  assert.strictEqual(s1.waits.length, 0);
  assert.strictEqual(ctx.panelState(), s1);

  // 2. The hub's state arrives: its tasks replace its cards; the other hub's card stays.
  ctx.panelOther.slug = 'hub1';
  ctx.panelOther.data = {
    tasks: [{ id: 'ht1', title: 'Task 1' }, { id: 'ht2', title: 'Task 2' }],
    gates: [{ id: 'g1', task: 'ht1' }],
    waits: [{ id: 'w1' }],
  };
  const s2 = ctx.panelState();
  assert.notStrictEqual(s2, s1);
  assert.strictEqual(s2.tasks.filter(t => t.id === 'ht1').length, 1);
  assert.strictEqual(byId(s2, 'ht1').title, 'Task 1');
  assert.strictEqual(byId(s2, 'ht2')._base, '/b/hub1');
  assert.strictEqual(byId(s2, 'hx')._slug, 'hub2');
  assert.strictEqual(s2.gates.length, 2);
  assert.strictEqual(s2.gates.find(g => g.id === 'g1')._slug, 'hub1');
  assert.strictEqual(s2.waits.length, 1);
  assert.strictEqual(s2.waits[0]._slug, 'hub1');
  assert.strictEqual(ctx.panelState(), s2);
});

test('withPanelState sets stateNow() to panelState() inside callback only', () => {
  const localState = {
    tasks: [{ id: 'local1' }],
    gates: [],
    waits: [],
    hubTasks: [{ id: 'ht1', ownerHub: { slug: 'hub1' } }],
  };
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'ht1' },
    state: localState,
  });

  assert.strictEqual(ctx.stateNow(), localState);
  ctx.withPanelState(() => {
    const active = ctx.stateNow();
    assert.notStrictEqual(active, localState);
    assert.strictEqual(active.tasks.find(t => t.id === 'ht1')._slug, 'hub1');
  });
  assert.strictEqual(ctx.stateNow(), localState);
});

test('openCardTask opens in place on resident multiBoard and does not call onBoard', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: null },
    state: { tasks: [], hubTasks: [{ id: 'c1', ownerHub: { slug: 'hub1' } }, { id: 'c2', ownerHub: { slug: 'hub1' } }] },
  });

  const cardLive = {
    id: 'c1',
    status: 'dispatched',
    ownerHub: { slug: 'hub1' },
  };
  ctx.openCardTask(cardLive, null);
  assert.deepStrictEqual(ctx.calls, [['openTaskPanel', 'c1', 'term']]);

  ctx.calls.length = 0;
  const cardBacklog = {
    id: 'c2',
    status: 'backlog',
    ownerHub: { slug: 'hub1' },
  };
  ctx.openCardTask(cardBacklog, { dummy: 1 });
  assert.strictEqual(ctx.calls.length, 2);
  assert.strictEqual(ctx.calls[0][0], 'openTaskDetail');
  assert.strictEqual(ctx.calls[0][1].id, 'c2');
  assert.strictEqual(ctx.calls[0][1]._slug, 'hub1');
  assert.strictEqual(ctx.calls[0][1]._base, '/b/hub1');
  assert.strictEqual(ctx.calls[1][0], 'boardApi'); // from refreshPanelOther(true)
  assert.strictEqual(ctx.calls[1][1], '/b/hub1');
});

test('openCardTask in scopeAll or non-resident server delegates to onBoard', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'all', view: 'agent', task: null },
    state: { tasks: [], hubTasks: [] },
  });

  const cardLive = {
    id: 'c1',
    status: 'dispatched',
    ownerHub: { slug: 'hub1' },
  };
  ctx.openCardTask(cardLive, null);
  assert.ok(ctx.calls.some(c => c[0] === 'onBoard' && c[1] === 'hub1'));

  ctx.calls.length = 0;
  ctx.multiBoard = false;
  ctx.nav.board = 'repo1';
  ctx.openCardTask(cardLive, null);
  assert.ok(ctx.calls.some(c => c[0] === 'onBoard' && c[1] === 'hub1'));
});

test('two hubs with the same task id: the pressed one wins and switchBoard clears the pick', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'dup1' },
    state: {
      tasks: [],
      hubTasks: [
        { id: 'dup1', ownerHub: { slug: 'hubA' } },
        { id: 'dup1', ownerHub: { slug: 'hubB' } },
      ],
    },
  });

  // Without a pressed card pick, falls back to the first match
  assert.strictEqual(ctx.panelOwnerSlug('dup1'), 'hubA');

  // Pressing card from hubB sets panelOwnerPick
  const cardB = {
    id: 'dup1',
    status: 'dispatched',
    ownerHub: { slug: 'hubB' },
  };
  ctx.openCardTask(cardB, null);
  assert.strictEqual(ctx.panelOwnerPick?.slug, 'hubB');
  assert.strictEqual(ctx.panelOwnerSlug('dup1'), 'hubB');

  // Switching boards clears the pick (and the page's state with it).
  ctx.switchBoard();
  assert.strictEqual(ctx.panelOwnerPick, null);
});

test('panelState fallback does not stick to first card when switching tasks before hub data arrives', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'tA' },
    state: {
      tasks: [],
      hubTasks: [
        { id: 'tA', ownerHub: { slug: 'hub1' } },
        { id: 'tB', ownerHub: { slug: 'hub1' } },
      ],
    },
  });

  // Before the hub's state arrives, every card of the hub is there, so moving from tA to tB finds tB.
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'tA')._slug, 'hub1');
  ctx.nav.task = 'tB';
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'tB')._slug, 'hub1');
});

test('panelState puts the panel hub\'s card first when two hubs share a task id', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'dup1' },
    state: { tasks: [], hubTasks: [{ id: 'dup1', ownerHub: { slug: 'hubA' } }, { id: 'dup1', ownerHub: { slug: 'hubB' } }] },
  });
  ctx.openCardTask({ id: 'dup1', status: 'dispatched', ownerHub: { slug: 'hubB' } }, null);
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'dup1')._slug, 'hubB');
});

test('panelOwnerSlug applies to agent (board) view only, returning null in work and todo views', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 't1' },
    state: {
      tasks: [],
      hubTasks: [{ id: 't1', ownerHub: { slug: 'hub1' } }],
    },
  });

  // In agent view: resolves owner
  assert.strictEqual(ctx.panelOwnerSlug('t1'), 'hub1');

  // In work view: null
  ctx.nav.view = 'work';
  assert.strictEqual(ctx.panelOwnerSlug('t1'), null);

  // In todo view: null
  ctx.nav.view = 'todo';
  assert.strictEqual(ctx.panelOwnerSlug('t1'), null);
});

test('when repository task and clicked hub card share an id, hub task wins in panelState', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'shared1' },
    state: {
      tasks: [{ id: 'shared1', title: 'Local repo task' }],
      hubTasks: [{ id: 'shared1', title: 'Hub task', ownerHub: { slug: 'hub1' } }],
    },
  });

  // Before clicking, the local task would be seen because panelOwnerSlug is null
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), null);
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'shared1').title, 'Local repo task');

  // Once the hub card is clicked, panelOwnerPick identifies hub1 as owner
  ctx.openCardTask({ id: 'shared1', status: 'dispatched', ownerHub: { slug: 'hub1' } }, null);
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), 'hub1');

  // Before hub data arrives: hub card comes before page's own tasks
  const beforeData = ctx.panelState().tasks.find(t => t.id === 'shared1');
  assert.strictEqual(beforeData._slug, 'hub1');
  assert.strictEqual(beforeData.title, 'Hub task');

  // After hub data arrives: hub task from data comes before page's own tasks
  ctx.panelOther.data = { tasks: [{ id: 'shared1', title: 'Hub task from api' }] };
  ctx.panelOther.slug = 'hub1';
  const afterData = ctx.panelState().tasks.find(t => t.id === 'shared1');
  assert.strictEqual(afterData._slug, 'hub1');
  assert.strictEqual(afterData.title, 'Hub task from api');
});

test('once the picked hub\'s data no longer lists nav.task, another hub card with same id is excluded from cards', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'shared1' },
    state: {
      tasks: [],
      hubTasks: [
        { id: 'shared1', ownerHub: { slug: 'hubA' } },
        { id: 'shared1', ownerHub: { slug: 'hubB' } },
      ],
    },
  });

  ctx.openCardTask({ id: 'shared1', status: 'dispatched', ownerHub: { slug: 'hubA' } }, null);
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), 'hubA');

  // Before data arrives, hubA card is present
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'shared1')._slug, 'hubA');

  // When hubA data arrives but does NOT list shared1, hubB card must not stand in
  ctx.panelOther.data = { tasks: [{ id: 'otherTask' }] };
  ctx.panelOther.slug = 'hubA';
  assert.strictEqual(ctx.panelState().tasks.find(t => t.id === 'shared1'), undefined);
});

test('pressing a card of this board drops the hub picked for the same id', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'shared1' },
    state: {
      tasks: [{ id: 'shared1', status: 'dispatched' }],
      hubTasks: [{ id: 'shared1', ownerHub: { slug: 'hub1' } }],
    },
  });
  ctx.openCardTask({ id: 'shared1', status: 'dispatched', ownerHub: { slug: 'hub1' } }, null);
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), 'hub1');
  ctx.openCardTask({ id: 'shared1', status: 'dispatched' }, null);
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), null);
});

test('when the picked hub\'s card leaves the page, another hub\'s card of the same id does not stand in', () => {
  const ctx = createContext({
    multiBoard: true,
    nav: { board: 'repo1', view: 'agent', task: 'shared1' },
    state: {
      tasks: [],
      hubTasks: [{ id: 'shared1', ownerHub: { slug: 'hubA' } }, { id: 'shared1', ownerHub: { slug: 'hubB' } }],
    },
  });
  ctx.openCardTask({ id: 'shared1', status: 'dispatched', ownerHub: { slug: 'hubA' } }, null);
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), 'hubA');
  ctx.state.hubTasks = [{ id: 'shared1', ownerHub: { slug: 'hubB' } }];
  assert.strictEqual(ctx.panelOwnerSlug('shared1'), null);
});
