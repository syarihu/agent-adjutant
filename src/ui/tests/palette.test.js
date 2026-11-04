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
  // The page's rule (org.js): an organisation the page does not list is shown under every selection.
  orgInScope: (scope, nwo) => scope.current == null || !scope.known.has(nwo.split('/')[0]) || nwo.split('/')[0] === scope.current,
});
vm.runInContext(read('my-work.js').match(/^function workRowOrder[\s\S]*?^}/m)[0], ctx);
for (const f of ['util.js', 'sessions.js']) vm.runInContext(read(f), ctx, { filename: f });
vm.runInContext(pure, ctx, { filename: 'palette.js' });
const { paletteEntries, paletteFilter, isPaletteShortcut, paletteStatusText, paletteActiveKey, paletteUnreadRepos } = ctx;
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

test('sessions waiting on the person come first, 確認待ち before 入力待ち; the rest by repository, then by task', () => {
  const mk = (id, state, task) => ({ board: 'a', session: w(id, { state }), task });
  const d = { repos: [
    { nwo: 'acme/app', rows: [mk('w1', 'working', { id: 't9', title: 'nine' }), mk('p1', 'permission'), mk('d1', 'done', { id: 't2', title: 'two' }), mk('q1', 'waiting'), mk('w2', 'working'), mk('s1', 'stopped', { id: 't5', title: 'five' })] },
    { nwo: 'acme/zed', rows: [{ board: 'z', session: w('z1', { state: 'working' }), task: { id: 't1', title: 'one' } }, { board: 'z', session: w('z2', { state: 'permission' }) }] },
  ] };
  // Waiting, then permission (document order within), then repository by repository: tasks by id, sessions with no task after them by key.
  assert.deepStrictEqual(keys(rowsOf(d)), ['a/sess:q1', 'a/sess:p1', 'z/sess:z2', 'a/t2', 'a/t5', 'a/t9', 'a/sess:w2', 'z/t1']);
  // Without any waiting row only the rest's order applies.
  assert.deepStrictEqual(keys(rowsOf(doc)), ['a/t1', 'a/sess:s2', 'b/t9']);
});

test('a repository listed first in the document stays first among the rest, whatever its tasks\' ids', () => {
  const d = { repos: [
    { nwo: 'acme/zzz', rows: [{ board: 'a', session: w('a1'), task: { id: 't9', title: 'late id' } }] },
    { nwo: 'acme/aaa', rows: [{ board: 'b', session: w('b1'), task: { id: 't1', title: 'early id' } }] },
  ] };
  assert.deepStrictEqual(keys(rowsOf(d)), ['a/t9', 'b/t1']);
});

test('the order holds when the query filters', () => {
  const mk = (id, state, branch) => ({ board: 'a', session: w(id, { state, branch }) });
  const d = { repos: [{ nwo: 'acme/app', rows: [mk('w1', 'working', 'fix/a'), mk('p1', 'permission', 'fix/b'), mk('x1', 'working', 'other'), mk('q1', 'waiting', 'fix/c')] }] };
  const es = rowsOf(d);
  assert.deepStrictEqual(keys(paletteFilter(es, '')), ['a/sess:q1', 'a/sess:p1', 'a/sess:w1', 'a/sess:x1']);
  assert.deepStrictEqual(keys(paletteFilter(es, 'FIX/')), ['a/sess:q1', 'a/sess:p1', 'a/sess:w1']);
});

test('a board served alone keeps the sessions\' order', () => {
  const st = { ...state0, sessions: [w('s2'), w('s1')], tasks: [] };
  assert.deepStrictEqual(keys(paletteEntries({ multi: false, doc: null, scope: scope(null), state: st })), ['self/s2', 'self/s1']);
});

test('the status: a stale list is said beside the result, never in its place; loading or failed with nothing read stands alone', () => {
  const base = { notice: '', total: 3, found: 3, drawn: 3, stale: false };
  assert.strictEqual(paletteStatusText(base), '');
  assert.strictEqual(paletteStatusText({ ...base, found: 0, drawn: 0 }), '該当するセッションがありません');
  assert.strictEqual(paletteStatusText({ ...base, total: 0, found: 0, drawn: 0 }), 'セッションがありません');
  assert.strictEqual(paletteStatusText({ ...base, found: 105, drawn: 100 }), 'ほか 5 件があります。言葉を足して絞り込んでください');
  const stale = '最新の状態を読めませんでした（前回の一覧です）';
  assert.strictEqual(paletteStatusText({ ...base, stale: true }), stale);
  assert.strictEqual(paletteStatusText({ ...base, found: 0, drawn: 0, stale: true }), `該当するセッションがありません ${stale}`);
  assert.strictEqual(paletteStatusText({ ...base, found: 105, drawn: 100, stale: true }), `ほか 5 件があります。言葉を足して絞り込んでください ${stale}`);
  assert.strictEqual(paletteStatusText({ ...base, notice: '読み込み中…', stale: true }), '読み込み中…');
});

test('repositories the work document could not read are named, in the selected organisation only', () => {
  const d = { repos: [{ nwo: 'acme/a', error: 'no board', rows: [] }, { nwo: 'acme/b', rows: [] }, { nwo: 'beta/c', error: 'x', rows: [] }, { nwo: 'ghost/d', error: 'y', rows: [] }] };
  assert.deepStrictEqual(plain(paletteUnreadRepos(d, scope('acme'))), ['acme/a', 'ghost/d']);
  assert.deepStrictEqual(plain(paletteUnreadRepos(d, scope('beta'))), ['beta/c', 'ghost/d']);
  assert.deepStrictEqual(plain(paletteUnreadRepos(d, scope(null))), ['acme/a', 'beta/c', 'ghost/d']);
  assert.deepStrictEqual(plain(paletteUnreadRepos(null, scope('acme'))), []);
});

test('the status names unreadable repositories beside the result, and a failed read is never "no sessions" alone', () => {
  const base = { notice: '', total: 0, found: 0, drawn: 0, stale: false };
  assert.strictEqual(paletteStatusText({ ...base, unread: ['acme/a'] }), 'セッションがありません 読めなかったリポジトリがあります: acme/a');
  assert.strictEqual(paletteStatusText({ ...base, total: 2, found: 2, drawn: 2, unread: ['acme/a', 'acme/b'] }), '読めなかったリポジトリがあります: acme/a, acme/b');
  assert.strictEqual(paletteStatusText({ ...base, total: 2, unread: ['a/1', 'a/2', 'a/3', 'a/4', 'a/5'] }), '該当するセッションがありません 読めなかったリポジトリがあります: a/1, a/2, a/3 ほか 2 件');
  assert.strictEqual(paletteStatusText({ ...base, stale: true, unread: ['acme/a'] }), 'セッションがありません 最新の状態を読めませんでした（前回の一覧です） 読めなかったリポジトリがあります: acme/a');
  assert.strictEqual(paletteStatusText({ ...base, notice: '読み込み中…', unread: ['acme/a'] }), '読み込み中…');
});

test('the highlight is always a drawn row: kept while drawn, else the first drawn row, else none', () => {
  const rows = ['a', 'b', 'c'].map(key => ({ key }));
  assert.strictEqual(paletteActiveKey(rows, 'b'), 'b');
  assert.strictEqual(paletteActiveKey(rows, 'gone'), 'a');
  assert.strictEqual(paletteActiveKey(rows, null), 'a');
  assert.strictEqual(paletteActiveKey([], 'b'), null);
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
