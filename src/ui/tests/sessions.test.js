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
const { ledgerState, sessionState, sessionTip, unknownWhy } = ctx;
const table = name => vm.runInContext(name, ctx);
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
