/* The state of a session as the page reads it (src/ui/sessions.js), run with `node --test`. The page's scripts
   share one global scope, so they are loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({ state: {}, BASE: '' });
for (const file of ['util.js', 'sessions.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const { ledgerState, sessionState, sessionTip, unknownWhy, passedGate, sessionLastText } = ctx;
const table = name => vm.runInContext(name, ctx);
const gateFirst = s => table('gateFirst')(s);
const plain = x => JSON.parse(JSON.stringify(x));

const NOW = 10_000;
// A `data` that is not the page's own `state`, so a restart in progress is not looked up.
const data = { now: NOW, hubs: [], sessions: [] };
const session = (extra = {}) => ({ id: 'w1', kind: 'worker', present: true, hub: 'hub', worktree: '/w/w1', ...extra });
const fresh = { lastActivityAt: NOW - 5 };
const quiet = { lastActivityAt: NOW - 600 };

test('a session with no row is unknown, whatever its window did', () => {
  assert.strictEqual(sessionState(session(fresh), data), 'unknown');
  assert.strictEqual(sessionState(session(quiet), data), 'unknown');
  assert.strictEqual(sessionState(session({ lastActivityAt: null }), data), 'unknown');
  assert.strictEqual(sessionState(session(), data), 'unknown');
});

test('a hub with no row is unknown', () => {
  assert.strictEqual(sessionState(session({ kind: 'hub', id: 'hub' }), data), 'unknown');
});

test('an unreadable ledger and an unknown status word are unknown', () => {
  assert.strictEqual(ledgerState(session({ ...fresh, agentSession: { error: 'boom' } }), data), 'unknown');
  assert.strictEqual(ledgerState(session({ ...fresh, agentSession: { status: 'brand-new' } }), data), 'unknown');
});

test('a running row defers to the pane', () => {
  const agentSession = { status: 'running' };
  assert.strictEqual(ledgerState(session({ ...fresh, agentSession }), data), 'working');
  assert.strictEqual(ledgerState(session({ ...quiet, agentSession }), data), 'idle');
});

test('the other statuses are the ledger\'s own', () => {
  const of = status => ledgerState(session({ ...fresh, agentSession: { status } }), data);
  assert.strictEqual(of('waiting'), 'permission');
  assert.strictEqual(of('idle'), 'done');
  assert.strictEqual(of('done'), 'done');
  assert.strictEqual(of('failed'), 'failed');
});

test('why a state is unknown tells the three causes apart', () => {
  assert.match(unknownWhy(session()), /adjutant setup claude/);
  assert.strictEqual(unknownWhy(session({ agentSession: { error: 'boom' } })), 'エージェントの状態を読めません');
  assert.strictEqual(unknownWhy(session({ agentSession: { status: 'brand-new' } })), 'エージェントの報告: brand-new');
  assert.strictEqual(unknownWhy(session({ agentSession: { status: 'idle' } })), '');
  const tip = sessionTip(session({ agentSession: { error: 'boom' } }), 'unknown', data);
  assert.ok(tip.includes('エージェントの状態を読めません'));
  assert.ok(!tip.includes('setup'));
  assert.ok(sessionTip(session(), 'unknown', data).includes('setup'));
});

test('every state has its words, icon, pill and row label', () => {
  for (const st of Object.keys(table('STATE_ORDER'))) {
    if (st === 'none' || st === 'pending') continue;
    for (const name of ['STATE_LABEL', 'STATE_ICON']) assert.ok(table(`${name}['${st}']`), `${name} ${st}`);
  }
  for (const st of ['unknown', 'idle']) {
    for (const name of ['STATE_LABEL', 'STATE_ICON', 'STATE_PILL', 'ROW_LABEL']) assert.ok(table(`${name}['${st}']`), `${name} ${st}`);
  }
  assert.strictEqual(table("STATE_LABEL['unknown']"), '状態不明');
});

// my-work.js draws into the page as it loads, so only its tables and pure helpers are cut out of the source
// and evaluated here.
const mySrc = fs.readFileSync(path.join(__dirname, '..', 'my-work.js'), 'utf8');
const cut = re => {
  const m = mySrc.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
vm.runInContext([
  cut(/^const WORK_LISTED = \[[^\]]*\];/m),
  cut(/^const WORK_RUNNING = \[[^\]]*\];/m),
  cut(/^const WORK_GLYPH = \{[\s\S]*?^\};/m),
  cut(/^const WORKS_ON_PERSON = [^\n]*;/m),
  cut(/^const workBoxOf = [^\n]*;/m),
].join('\n'), ctx);

test('an unknown session is listed under 実行中, and is neither work on a person nor working', () => {
  assert.ok(table("WORK_LISTED.includes('unknown')"));
  assert.strictEqual(table("workBoxOf({ st: 'unknown' })"), 'running');
  assert.deepStrictEqual(plain(table("WORK_GLYPH.unknown")), ['visibility_off', '状態不明']);
  assert.strictEqual(table("WORKS_ON_PERSON({ st: 'unknown' })"), false);
  assert.strictEqual(table("WORKS_ON_PERSON({ st: 'permission' })"), true);
});

test('the row pill of an unknown session does not read as running', () => {
  assert.strictEqual(table("ROW_LABEL['unknown']"), '不明');
  assert.notStrictEqual(table("ROW_LABEL['unknown']"), table("ROW_LABEL['working']"));
});

test('the away summary knows the same ledger statuses as the page', () => {
  const awaySrc = fs.readFileSync(path.join(__dirname, '..', 'my-work-away.js'), 'utf8');
  const m = awaySrc.match(/^const WORK_NOW_STATUSES = (\[[^\]]*\]);/m);
  assert.ok(m, 'WORK_NOW_STATUSES not found');
  assert.deepStrictEqual(plain(vm.runInNewContext(m[1])).sort(), plain(Object.keys(table('AGENT_STATES'))).sort());
});

// openSessionRef reads the page's globals when called; set the ones the scripts above do not define, then put them back.
const openWith = (over, ref = 'w1') => {
  const keys = ['multiBoard', 'view', 'nav', 'go', 'openTaskPanel', 'taskOfSession', 'hasSession', 'HUB_REF', 'SESS_REF', 'state'];
  const saved = Object.fromEntries(keys.map(k => [k, ctx[k]]));
  const calls = { go: [], panel: [] };
  Object.assign(ctx, {
    multiBoard: true, view: 'board', nav: { board: 'a', task: null }, HUB_REF: 'hub:', SESS_REF: 'session:',
    go: (patch, opts) => calls.go.push([patch, opts]),
    openTaskPanel: (id, pane) => calls.panel.push([id, pane]),
    taskOfSession: () => null, hasSession: () => true,
    state: { hubs: [{ id: 'hub', slug: 'a' }, { id: 'other', slug: 'b' }], sessions: [session()] },
  }, over);
  try {
    ctx.openSessionRef(ref);
  } finally {
    for (const k of keys) if (saved[k] === undefined) delete ctx[k]; else ctx[k] = saved[k];
  }
  return calls;
};

test('on a board of a resident server a session opens in that board\'s panel, on its terminal', () => {
  const c = openWith({});
  assert.deepStrictEqual(c.panel, [['session:w1', 'term']]);
  assert.strictEqual(c.go.length, 0);
});

test('a session whose hub is another board\'s still opens in the panel it is pressed on', () => {
  const c = openWith({ state: { hubs: [{ id: 'hub', slug: 'b' }], sessions: [session()] } });
  assert.deepStrictEqual(c.panel, [['session:w1', 'term']]);
  assert.strictEqual(c.go.length, 0);
});

test('in 「いまの仕事」 a session opens there, on the board of its hub', () => {
  const c = openWith({ view: 'work', state: { hubs: [{ id: 'hub', slug: 'b' }], sessions: [session()] } });
  assert.strictEqual(c.panel.length, 0);
  assert.strictEqual(c.go.length, 1);
  assert.deepStrictEqual(plain(c.go[0][0]), { board: 'b', view: 'work', task: 'session:w1', pane: 'detail' });
});

test('a board served alone opens the panel, on the terminal only when the session has one', () => {
  assert.deepStrictEqual(openWith({ multiBoard: false }).panel, [['session:w1', 'term']]);
  const c = openWith({ multiBoard: false, hasSession: () => false });
  assert.deepStrictEqual(c.panel, [['session:w1', 'detail']]);
  assert.strictEqual(c.go.length, 0);
});

test('a session that owns a task opens that task\'s panel', () => {
  const c = openWith({ taskOfSession: () => ({ id: 'T-1' }) });
  assert.deepStrictEqual(c.panel, [['T-1', 'term']]);
  assert.strictEqual(c.go.length, 0);
});

// A gate opened at G; `secsStamp` is not loaded here, so the stamps are written out.
const G = Date.UTC(2026, 9, 1) / 1000;
const stampAt = secs => new Date(secs * 1000).toISOString().replace(/[-:]|\.\d{3}/g, '');
const gated = (agentSession, extra = {}) => session({ ...fresh, waiting: { id: 'g1', kind: 'plan', openedAt: stampAt(G), count: 1 }, agentSession, ...extra });

test('a worker went on past its gate only by a status that began after the gate opened', () => {
  const past = (s, secs = G) => passedGate(s, secs, data);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60 })), true);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 25 })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G - 100 })), false);
  assert.strictEqual(past(gated({ status: 'done', updatedAt: G + 60 })), false);
  assert.strictEqual(past(gated({ status: 'idle', updatedAt: G + 60 })), false);
  assert.strictEqual(past(gated({ status: 'failed', updatedAt: G + 60 })), true);
  assert.strictEqual(past(gated({ status: 'waiting', request: 'Bash: make', updatedAt: G + 60 })), true);
  assert.strictEqual(past(gated({ status: 'waiting', request: 'AskUserQuestion: どちら？', updatedAt: G + 60 })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 31 })), true);
  // A `running` row whose pane went quiet is read as idle, so the worker has not gone on.
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60 }, quiet)), false);
  assert.strictEqual(past(gated({ status: 'running' })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60, error: 'boom' })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60 }, { present: false })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60 }, { kind: 'hub' })), false);
  assert.strictEqual(past(gated({ status: 'running', updatedAt: G + 60 }), null), false);
  assert.strictEqual(past(null), false);
});

test('a session is read by its gate first unless it went on past it', () => {
  const run = { status: 'running', updatedAt: G + 60 };
  assert.strictEqual(gateFirst(gated(run)), false);
  assert.strictEqual(gateFirst(gated({ status: 'done', updatedAt: G + 60 })), true);
  assert.strictEqual(gateFirst(gated(run, { waiting: { id: 'g1', kind: 'plan', openedAt: 'garbage', count: 1 } })), true);
  assert.strictEqual(gateFirst(gated(run, { waiting: { id: 'g1', kind: 'plan', openedAt: stampAt(G), count: 2 } })), true);
  assert.strictEqual(gateFirst(session()), false);
});

test('a session with a gate it went on past reads by its own state', () => {
  assert.strictEqual(sessionState(gated({ status: 'running', updatedAt: G + 60 }), data), 'working');
  assert.strictEqual(sessionState(gated({ status: 'running', updatedAt: G + 60 }, quiet), data), 'waiting');
  assert.strictEqual(sessionState(gated({ status: 'waiting', request: 'Bash: make', updatedAt: G + 60 }), data), 'permission');
  assert.strictEqual(sessionState(gated({ status: 'done', updatedAt: G + 60 }), data), 'waiting');
  const asks = status => sessionLastText(gated({ status, request: 'Bash: make', updatedAt: G + 60 }), data);
  assert.match(asks('waiting'), /Bash: make/);
  assert.doesNotMatch(asks('done'), /Bash: make/);
});

test('one rule for several gates: past the newest, and every time readable', () => {
  const s = gated({ status: 'running', updatedAt: G + 60 });
  const past = list => table('passedGates')(s, list, data);
  assert.strictEqual(past([G, G + 10]), true);
  assert.strictEqual(past([G, G + 50]), false);
  assert.strictEqual(past([G, null]), false);
  assert.strictEqual(past([]), false);
});

test('a task is past its gates only by the ones that wait on a person', () => {
  const task = { id: 't1' };
  const worker = session({ ...fresh, task: 't1', agentSession: { status: 'running', updatedAt: G + 60 } });
  const run = gates => {
    ctx.state = { gates, sessions: [worker], now: NOW };
    // The page's own lookup is in terminal.js; the task's worker is the only session here.
    ctx.sessionOfTask = t => ctx.state.sessions.find(x => x.task === t.id);
    return table('taskPassedGate')(task, ctx.state);
  };
  const plan = { task: 't1', openedAt: stampAt(G) };
  assert.strictEqual(run([plan]), true);
  assert.strictEqual(run([plan, { task: 't1', wait: false, openedAt: stampAt(G + 300) }]), true);
  assert.strictEqual(run([{ ...plan, answeredByHub: true }]), false);
  assert.strictEqual(run([]), false);
});
