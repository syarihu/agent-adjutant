/* The session search's candidates and matching (src/ui/palette.js), run with `node --test`. Only its pure first half is
   loaded; the page's own helpers it reads come from util.js and sessions.js, the rest is stubbed. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const pure = read('palette.js').split('/* ── drawing and keys ── */')[0];
assert.ok(pure.includes('function paletteFilter'), 'the marker moved');

const state0 = { now: 1000, hubs: [], sessions: [], tasks: [], parents: [], repo: 'acme/app' };
const ctx = vm.createContext({
  navigator: { platform: '' }, state: state0, BASE: '', scopeAll: () => false,
  WORK_LISTED: ['waiting', 'permission', 'working', 'idle', 'unknown', 'done', 'failed', 'stopped', 'restarting'],
  // The work document's rows: each repo's rows as they come, in order (the page's own flattening is tested with my-work.js).
  workRows: doc => (doc?.repos || []).flatMap(repo => (repo.rows || []).map(r => ({
    key: `${r.board}/${r.task ? r.task.id : 'sess:' + r.session.id}`, board: r.board, ref: r.task ? r.task.id : 'sess:' + r.session.id,
    repo, nwo: repo.nwo, s: r.session, st: r.session.state, data: { now: 1, hubs: repo.hubs || [], sessions: [] }, task: r.task || null, hubSlug: r.board,
  }))),
  orgInScope: (scope, nwo) => scope.current == null || nwo.split('/')[0] === scope.current,
});
for (const f of ['util.js', 'sessions.js']) vm.runInContext(read(f), ctx, { filename: f });
vm.runInContext(pure, ctx, { filename: 'palette.js' });
const { paletteEntries, paletteFilter, isPaletteShortcut } = ctx;
const plain = v => JSON.parse(JSON.stringify(v));
const keys = es => plain([...es].map(e => e.key));

const w = (id, extra = {}) => ({ id, kind: 'worker', present: true, hub: 'h', worktree: `/w/${id}`, state: 'working', ...extra });
const doc = {
  repos: [
    { nwo: 'acme/app', hubs: [], parents: [{ key: 'ACME-1', number: 1, title: 'Billing rework' }], rows: [
      { board: 'a', session: w('s1', { branch: 'feature/login-form' }), task: { id: 't1', title: 'Fix Login Bug', issue: 'acme/app#42', parent: 'ACME-1', worktreeName: 'login-wt' } },
      { board: 'a', session: w('s2', { state: 'stopped' }) },
    ] },
    { nwo: 'beta/site', hubs: [], parents: [], rows: [
      // The same session id as acme's, on another board.
      { board: 'b', session: w('s1', { branch: 'main' }), task: { id: 't9', title: 'Update footer' } },
    ] },
  ],
};
const scope = current => ({ current, known: new Set(['acme', 'beta']) });
const rowsOf = (d = doc, sc = scope(null)) => paletteEntries({ multi: true, doc: d, scope: sc, state: state0 });

test('resident server: candidates are the work rows, in the list\'s order', () => {
  assert.deepStrictEqual(keys(rowsOf()), ['a/t1', 'a/sess:s2', 'b/t9']);
});

test('keys are board plus ref: one session id on two boards stays two entries', () => {
  const es = rowsOf();
  assert.strictEqual(new Set(keys(es)).size, es.length);
  const dup = { repos: [{ nwo: 'acme/app', rows: [{ board: 'a', session: w('s1') }, { board: 'a', session: w('s1') }] }] };
  assert.strictEqual(rowsOf(dup).length, 1);
  const two = { repos: [{ nwo: 'acme/app', rows: [{ board: 'a', session: w('s1') }] }, { nwo: 'acme/b', rows: [{ board: 'b', session: w('s1') }] }] };
  assert.deepStrictEqual(keys(rowsOf(two)), ['a/sess:s1', 'b/sess:s1']);
});

test('org scope: only the selected organisation\'s rows', () => {
  assert.deepStrictEqual(keys(rowsOf(doc, scope('acme'))), ['a/t1', 'a/sess:s2']);
  assert.deepStrictEqual(keys(rowsOf(doc, scope('beta'))), ['b/t9']);
});

test('no document: no candidates (the caller says why)', () => {
  assert.deepStrictEqual(plain(rowsOf(null)), []);
});

test('a board served alone: state.sessions in the listed states, keyed apart from the resident\'s', () => {
  const st = { ...state0, sessions: [w('s1', { branch: 'fix/x', task: 't1' }), w('s2', { present: false })],
    tasks: [{ id: 't1', title: 'Solo task' }] };
  const es = paletteEntries({ multi: false, doc: null, scope: scope(null), state: st });
  assert.ok(es.length >= 1);
  assert.ok(es.every(e => e.key.startsWith('self/') && e.sid && !e.row));
  assert.ok(es.some(e => e.title === 'Solo task'));
});

test('empty query keeps everything in order; blank space too', () => {
  const es = rowsOf();
  assert.deepStrictEqual(keys(paletteFilter(es, '')), keys(es));
  assert.deepStrictEqual(keys(paletteFilter(es, '   ')), keys(es));
  assert.deepStrictEqual(keys(paletteFilter(es, undefined)), keys(es));
});

test('matching is case-insensitive and every word must match, in any order', () => {
  const es = rowsOf();
  assert.deepStrictEqual(keys(paletteFilter(es, 'LOGIN')), ['a/t1']);
  assert.deepStrictEqual(keys(paletteFilter(es, 'bug login')), ['a/t1']);
  assert.deepStrictEqual(keys(paletteFilter(es, 'login footer')), []);
});

test('each field is searchable: title, issue key and number, branch, repository, parent, worktree', () => {
  const es = rowsOf();
  const hit = q => keys(paletteFilter(es, q));
  assert.deepStrictEqual(hit('footer'), ['b/t9']);          // title
  assert.deepStrictEqual(hit('t9'), ['b/t9']);              // task id
  assert.deepStrictEqual(hit('acme/app#42'), ['a/t1']);     // issue key
  assert.deepStrictEqual(hit('#42'), ['a/t1']);             // issue number
  assert.deepStrictEqual(hit('feature/login'), ['a/t1']);   // branch
  assert.deepStrictEqual(hit('beta/site'), ['b/t9']);       // repository
  assert.deepStrictEqual(hit('acme-1'), ['a/t1']);          // parent key
  assert.deepStrictEqual(hit('billing'), ['a/t1']);         // parent title
  assert.deepStrictEqual(hit('login-wt'), ['a/t1']);        // worktree name of the task
  assert.deepStrictEqual(hit('s2'), ['a/sess:s2']);         // the session's own worktree
});

test('entries carry what the row draws', () => {
  const e = plain(rowsOf()[0]);
  assert.strictEqual(e.title, 'Fix Login Bug');
  assert.ok(e.sub.includes('feature/login-form') && e.sub.includes('acme/app'));
  assert.strictEqual(e.st, 'working');
});

test('the shortcut: Cmd+K on macOS, Ctrl+K elsewhere, never the other, never with Alt or Shift or mid-conversion', () => {
  const k = (extra = {}) => ({ type: 'keydown', key: 'k', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...extra });
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true }), true), true);
  assert.strictEqual(isPaletteShortcut(k({ key: 'K', metaKey: true }), true), true);
  assert.strictEqual(isPaletteShortcut(k({ ctrlKey: true }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, ctrlKey: true }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ ctrlKey: true }), false), true);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true }), false), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, ctrlKey: true }), false), false);
  assert.strictEqual(isPaletteShortcut(k(), true), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, altKey: true }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ ctrlKey: true, shiftKey: true }), false), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, isComposing: true }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, keyCode: 229 }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, key: 'j' }), true), false);
  assert.strictEqual(isPaletteShortcut(k({ metaKey: true, type: 'keyup' }), true), false);
});
