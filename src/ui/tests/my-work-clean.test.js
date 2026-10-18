/* 「片付ける」 on a merged row (src/ui/my-work-clean.js), run with `node --test`.
   The file reads the page's globals and starts requests, so the pure helpers and the press are cut out and run against stubs. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'my-work-clean.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const plain = v => JSON.parse(JSON.stringify(v));
const calls = [];
let reply = null;
const ctx = vm.createContext({
  esc: x => String(x).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'),
  baseName: p => (p || '').split('/').filter(Boolean).pop() || '',
  enc: encodeURIComponent, view: 'work', calls,
  workMerged: r => r.task ? r.task.prState === 'merged' : r.s?.branchPr?.state === 'merged',
  boardApi: async (base, p, o) => { calls.push(['api', base, p, o?.method || 'GET', o?.body]); if (reply instanceof Error) throw reply; return reply; },
  note: (line, bad, why) => calls.push(['note', bad, why]),
  go: a => calls.push(['go', a]), refreshWork: () => calls.push(['refreshWork']), renderWorkList: () => {},
  cleanupSummary: () => ({ text: 'removed', bad: false }),
  openCleanupDialog: (s, base, o) => calls.push(['dialog', s.id, base, o]),
  workIsSelected: r => !!r.selected,
});
vm.runInContext([
  cut(/^const workClean = [^\n]*;/m),
  cut(/^const WORK_CLEAN_TTL = [^\n]*;/m),
  cut(/^const workCleanKey = [^\n]*;/m),
  cut(/^function workCleanFacts[\s\S]*?^}/m),
  cut(/^function workCleanLineHtml[\s\S]*?^}/m),
  cut(/^const workCleanRowHtml = [^\n]*;/m),
  cut(/^const workCleanable = [^\n]*;/m),
  cut(/^function workCleanWant[\s\S]*?^}/m),
  cut(/^async function workCleanUp[\s\S]*?^}/m),
].join('\n'), ctx);
const run = (code, vars = {}) => { Object.assign(ctx, vars); return vm.runInContext(code, ctx); };
const row = (extra = {}) => ({ key: 'b1/t1', board: 'b1', s: { id: 's1', kind: 'worker', worktree: '/w/tree', uncommitted: { files: 9, untracked: 9 } }, task: { id: 't1', prState: 'merged' }, ...extra });
const clean = { uncommitted: { files: 0, untracked: 0 }, unpushed: { count: 0 } };
ctx.clean = clean;

test('facts: all zero from a git reply is ok; untracked only, or unpushed commits, is not', () => {
  const facts = (u, git) => plain(run('workCleanFacts(u, git)', { u, git }));
  assert.deepStrictEqual(facts(null, clean), { files: 0, unpushed: 0, ok: true });
  assert.strictEqual(facts(null, { ...clean, uncommitted: { files: 0, untracked: 2 } }).ok, false);
  assert.deepStrictEqual(facts(null, { ...clean, uncommitted: { files: 1, untracked: 2 } }).files, 3);
  assert.deepStrictEqual(facts(null, { ...clean, unpushed: { count: 2 } }), { files: 0, unpushed: 2, ok: false });
});

test('facts: without a git reply the row\'s own counts are shown and never make it ok', () => {
  const f = plain(run('workCleanFacts(u, null)', { u: { files: 0, untracked: 0 } }));
  assert.deepStrictEqual(f, { files: 0, unpushed: null, ok: false });
  assert.strictEqual(plain(run('workCleanFacts(null, { uncommitted: { files: 0, untracked: 0 } })')).ok, false);
});

test('line: reading says so, a failed read says the state cannot be confirmed, and neither has a button', () => {
  const line = e => run('workCleanLineHtml(r, e)', { r: row(), e });
  for (const html of [line(undefined), line({ reading: true, git: null })]) assert.ok(html.includes('確認しています…') && !html.includes('<button'));
  const failed = line({ error: 'boom' });
  assert.ok(failed.includes('git の状態を確認できません') && !failed.includes('<button'));
});

test('line: the button only when nothing would be lost; otherwise the counts and 片付けられません', () => {
  const line = git => run('workCleanLineHtml(r, e)', { r: row(), e: { git } });
  const ok = line(clean);
  assert.ok(ok.includes('未コミット 0 · 未 push 0') && ok.includes('data-wk-clean="b1/t1"') && ok.includes('>片付ける<'));
  const dirty = line({ ...clean, uncommitted: { files: 2, untracked: 1 } });
  assert.ok(dirty.includes('未コミット 3') && dirty.includes('片付けられません') && !dirty.includes('<button'));
  const unpushed = line({ ...clean, unpushed: { count: 2 } });
  assert.ok(unpushed.includes('未 push 2') && !unpushed.includes('<button'));
  // A clean-up the server refused after the read has no button either, whatever the read said.
  const blocked = run('workCleanLineHtml(r, e)', { r: row(), e: { git: clean, blocked: true } });
  assert.ok(blocked.includes('片付けられません') && !blocked.includes('<button'));
  assert.ok(run('workCleanLineHtml(r, e)', { r: row(), e: { blocked: true, reasons: [{ detail: '2 untracked path(s)' }] } }).includes('2 untracked path(s)'));
  assert.ok(run('workCleanLineHtml(r, e)', { r: row(), e: { failed: '<b>x</b>' } }).includes('&lt;b&gt;x&lt;/b&gt;'));
});

test('line: the button is outside any row button and is dimmed while the press is out', () => {
  const html = run('workCleanLineHtml(r, e, true)', { r: row(), e: { git: clean } });
  assert.ok(!html.includes('wk-row') && html.includes('disabled') && html.includes('片付けています…'));
});

test('only a merged worker\'s session with a worktree is cleanable', () => {
  const can = r => run('workCleanable(r)', { r });
  assert.ok(can(row()));
  assert.ok(!can(row({ task: { id: 't1', prState: 'open' } })));
  assert.ok(!can(row({ isHub: true })) && !can(row({ turn: true })));
  assert.ok(!can(row({ s: { id: 's1', kind: 'hub', worktree: '/w' } })) && !can(row({ s: { id: 's1', kind: 'worker' } })));
  assert.ok(can(row({ task: null, s: { id: 's1', kind: 'worker', worktree: '/w', branchPr: { state: 'merged' } } })));
});

test('a press posts the clean-up that loses nothing to the row\'s own board, and the row goes', async () => {
  calls.length = 0;
  run('workClean.git.clear(); workClean.busy.clear()');
  run('workClean.git.set("b1/s1", { at: 1, git: clean })');
  reply = { removed: true };
  await run('workCleanUp(r)', { r: row({ selected: true }) });
  assert.deepStrictEqual(plain(calls.filter(c => c[0] === 'api')), [['api', '/b/b1', '/api/sessions/s1/cleanup', 'POST', '{}']]);
  assert.ok(calls.some(c => c[0] === 'go') && calls.some(c => c[0] === 'refreshWork'));
  assert.strictEqual(run('workClean.git.has("b1/s1")'), false);
  assert.strictEqual(run('workClean.busy.size'), 0);
});

test('a refused clean-up keeps what it said, takes the button away and opens the dialog on the row\'s board', async () => {
  calls.length = 0;
  run('workClean.git.set("b1/s1", { at: 1, git: clean })');
  reply = { removed: false, git: { ...clean, unpushed: { count: 1 } }, reasons: [{ detail: 'x' }] };
  await run('workCleanUp(r)', { r: row() });
  assert.deepStrictEqual(plain(calls.find(c => c[0] === 'dialog')), ['dialog', 's1', '/b/b1', { blocked: true }]);
  assert.strictEqual(run('workClean.git.get("b1/s1").blocked'), true);
  assert.ok(!run('workCleanRowHtml(r)', { r: row() }).includes('<button'));
});

test('a failed request is a note and the row says so; nothing is pressed without a read that said it is clean', async () => {
  calls.length = 0;
  run('workClean.git.set("b1/s1", { at: 1, git: clean })');
  reply = new Error('nope');
  await run('workCleanUp(r)', { r: row() });
  assert.ok(calls.some(c => c[0] === 'note' && c[1] === true));
  assert.ok(run('workCleanRowHtml(r)', { r: row() }).includes('片付けられませんでした: nope'));
  calls.length = 0;
  run('workClean.git.set("b1/s1", { at: 1, git: { ...clean, unpushed: { count: 1 } } })');
  await run('workCleanUp(r)', { r: row() });
  run('workClean.git.clear()');
  await run('workCleanUp(r)', { r: row() });
  assert.strictEqual(calls.filter(c => c[0] === 'api').length, 0);
});

test('a second press while one is out is ignored', async () => {
  calls.length = 0;
  run('workClean.git.set("b1/s1", { at: 1, git: clean })');
  run('workClean.busy.add("b1/s1")');
  await run('workCleanUp(r)', { r: row() });
  assert.strictEqual(calls.length, 0);
  run('workClean.busy.clear()');
});
