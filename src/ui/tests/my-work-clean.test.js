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
  prRefNumber: pr => /(\d+)$/.exec(pr || '')?.[1] || '',
  qcIcon: n => `<span class="material-symbols-outlined" aria-hidden="true">${n}</span>`,
  pageRows: [],
  workRows: () => ctx.pageRows,
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
  cut(/^function workCleanClicked[\s\S]*?^}/m),
  cut(/^async function workCleanUp[\s\S]*?^}/m),
  cut(/^const workAsk = [^\n]*;/m),
  cut(/^const workAskKey = [^\n]*;/m),
  cut(/^function workMergedAsk\(r\)[\s\S]*?^}/m),
  cut(/^const workEntryGated = [^\n]*;/m),
  cut(/^function workMergedAskOf[\s\S]*?^}/m),
  cut(/^function workAskPrune[\s\S]*?^}/m),
  cut(/^const workMergedAskText = [^\n]*;/m),
  cut(/^function workMergedAskHtml[\s\S]*?^}/m),
  cut(/^async function workKeep[\s\S]*?^}/m),
  cut(/^async function workAskPress[\s\S]*?^}/m),
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
  assert.ok(failed.includes('class="wk-clean bad"') && failed.includes('warning'));
  for (const html of [line(undefined), line({ reading: true, git: null })]) assert.ok(!html.includes('bad'));
});

test('line: the button only when nothing would be lost; otherwise the counts and 片付けられません', () => {
  const line = git => run('workCleanLineHtml(r, e)', { r: row(), e: { git } });
  const ok = line(clean);
  assert.ok(ok.includes('未コミット 0 · 未 push 0') && ok.includes('data-wk-clean="b1/t1"') && ok.includes('>片付ける<') && ok.includes('cleaning_services') && !ok.includes('bad'));
  const dirty = line({ ...clean, uncommitted: { files: 2, untracked: 1 } });
  assert.ok(dirty.includes('未コミット 3') && dirty.includes('片付けられません') && !dirty.includes('<button'));
  assert.ok(dirty.includes('class="wk-clean bad"') && dirty.includes('warning'));
  const unpushed = line({ ...clean, unpushed: { count: 2 } });
  assert.ok(unpushed.includes('未 push 2') && !unpushed.includes('<button'));
  // A clean-up the server refused after the read has no button either, whatever the read said.
  const blocked = run('workCleanLineHtml(r, e)', { r: row(), e: { git: clean, blocked: true } });
  assert.ok(blocked.includes('片付けられません') && !blocked.includes('<button'));
  assert.ok(blocked.includes('class="wk-clean bad"') && blocked.includes('warning'));
  assert.ok(run('workCleanLineHtml(r, e)', { r: row(), e: { blocked: true, reasons: [{ detail: '2 untracked path(s)' }] } }).includes('2 untracked path(s)'));
  assert.ok(run('workCleanLineHtml(r, e)', { r: row(), e: { failed: '<b>x</b>' } }).includes('&lt;b&gt;x&lt;/b&gt;'));
  assert.ok(run('workCleanLineHtml(r, e)', { r: row(), e: { failed: 'x' } }).includes('class="wk-clean bad"'));
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

test('the button\'s click finds its row by the key it carries and cleans that one, on its own board; a dimmed button does nothing', async () => {
  calls.length = 0;
  run('workClean.git.clear(); workClean.busy.clear()');
  const one = row(), two = row({ key: 'b2/t2', board: 'b2', s: { id: 's2', kind: 'worker', worktree: '/w/two' }, task: { id: 't2', prState: 'merged' } });
  run('workClean.git.set("b1/s1", { at: 1, git: clean }); workClean.git.set("b2/s2", { at: 1, git: clean })');
  reply = { removed: true };
  const rowsByKey = new Map([[one.key, one], [two.key, two]]);
  await run('workCleanClicked(b, rows)', { b: { dataset: { wkClean: 'b2/t2' }, disabled: false }, rows: rowsByKey });
  assert.deepStrictEqual(plain(calls.filter(c => c[0] === 'api')), [['api', '/b/b2', '/api/sessions/s2/cleanup', 'POST', '{}']]);
  calls.length = 0;
  await run('workCleanClicked(b, rows)', { b: { dataset: { wkClean: 'b1/t1' }, disabled: true }, rows: rowsByKey });
  await run('workCleanClicked(b, rows)', { b: { dataset: { wkClean: 'nowhere' }, disabled: false }, rows: rowsByKey });
  assert.strictEqual(calls.length, 0);
});

/* ── the board asks about a merged session from the document's facts ── */

const merged = (extra = {}, task = { id: 't1', title: 'Retry <upload>', prState: 'merged', pr: 'https://github.com/acme/widget/pull/12' }) => ({
  task, s: { id: 's1', kind: 'worker', worktree: '/w/tree', uncommitted: { files: 0, untracked: 0 }, unpushed: 0, ...extra },
});
const ask = r => plain(run('workMergedAsk(r)', { r }));

test('ask: nothing to lose is clean, work to lose is dirty with both counts, and the PR number and title come from the task', () => {
  assert.deepStrictEqual(ask(merged()), { ask: 'clean', files: 0, unpushed: 0, number: '12', title: 'Retry <upload>' });
  assert.deepStrictEqual(ask(merged({ uncommitted: { files: 2, untracked: 1 } })).ask, 'dirty');
  assert.deepStrictEqual(ask(merged({ uncommitted: { files: 2, untracked: 1 }, unpushed: 4 })), { ask: 'dirty', files: 3, unpushed: 4, number: '12', title: 'Retry <upload>' });
  assert.strictEqual(ask(merged({ unpushed: 1 })).ask, 'dirty');
  assert.strictEqual(ask(merged({ uncommitted: { files: 0, untracked: 5 } })).ask, 'dirty');
});

test('ask: a fact that is not known, or a read that failed, is no answer', () => {
  assert.strictEqual(ask(merged({ unpushed: undefined })), null);
  assert.strictEqual(ask(merged({ uncommitted: undefined })), null);
  assert.strictEqual(ask(merged({ uncommitted: { files: 0 } })), null);
  assert.strictEqual(ask(merged({ uncommittedError: 'git could not read HEAD' })), null);
  assert.strictEqual(ask(merged({ unpushed: null })), null);
});

test('ask: only a merged worker session with a worktree; a branch PR stands in for a task\'s', () => {
  assert.strictEqual(ask(merged({}, { id: 't1', prState: 'open' })), null);
  assert.strictEqual(ask({ ...merged(), isHub: true }), null);
  assert.strictEqual(ask(merged({ kind: 'hub' })), null);
  assert.strictEqual(ask(merged({ worktree: '' })), null);
  assert.strictEqual(ask(merged({ id: '' })), null);
  const branch = merged({ branchPr: { state: 'merged', number: 7 }, taskTitle: 'From the record' }, null);
  assert.deepStrictEqual(ask(branch), { ask: 'clean', files: 0, unpushed: 0, number: '7', title: 'From the record' });
  assert.strictEqual(ask(merged({ branchPr: { state: 'open' } }, null)), null);
  // An entry holds the session as `session`.
  assert.strictEqual(ask({ task: merged().task, session: merged().s }).ask, 'clean');
  // No number: the title is the session's, or the worktree's name.
  assert.strictEqual(ask(merged({ branchPr: { state: 'merged' }, title: 'Own title' }, null)).title, 'Own title');
  assert.strictEqual(ask(merged({ branchPr: { state: 'merged' } }, null)).title, 'tree');
});

test('ask text: the number when there is one, the title only where the session is not named already', () => {
  const text = (a, titled) => run('workMergedAskText(a, titled)', { a, titled });
  assert.strictEqual(text({ number: '12', title: 'T' }, false), 'PR #12 マージ済み。片付けてセッションを閉じる？');
  assert.strictEqual(text({ number: '12', title: 'T' }, true), 'T: PR #12 マージ済み。片付けてセッションを閉じる？');
  assert.strictEqual(text({ number: '', title: 'T' }, false), 'マージ済み。片付けてセッションを閉じる？');
  assert.strictEqual(text({ number: '', title: 'T' }, true), 'T: マージ済み。片付けてセッションを閉じる？');
});

const entry = (extra = {}) => ({ board: 'b1', task: merged().task, session: merged().s, gates: [], ...extra });
const asked = (e, kept = new Set(), isDone = () => false) => plain(run('workMergedAskOf(e, { kept, isDone })', { e, kept, isDone }));

test('asked: a gate of the entry, or one the session waits on, leaves only the gate; one answered here no longer does', () => {
  assert.strictEqual(asked(entry()).ask, 'clean');
  assert.strictEqual(asked(entry({ gates: [{ key: 'b1/g1' }] })), null);
  const waiting = { ...merged().s, waiting: { slug: 'b1', id: 'g1' } };
  assert.strictEqual(asked(entry({ session: waiting })), null);
  assert.strictEqual(asked(entry({ session: waiting }), new Set(), key => key === 'b1/g1').ask, 'clean');
  // The dirty line is a question to nobody, but a gate is still the only thing asked.
  assert.strictEqual(asked(entry({ session: { ...merged().s, unpushed: 1 }, gates: [{ key: 'b1/g1' }] })), null);
});

test('asked: 「残す」 hides the clean question, by the record or by the page until the document says it, and not the dirty line', () => {
  assert.strictEqual(asked(entry({ session: { ...merged().s, cleanupKeptAt: 1700000000 } })), null);
  assert.strictEqual(asked(entry(), new Set(['clean:b1/s1'])), null);
  assert.strictEqual(asked(entry(), new Set(['clean:b2/s1'])).ask, 'clean');
  assert.strictEqual(asked(entry({ session: { ...merged().s, unpushed: 2, cleanupKeptAt: 1 } })).ask, 'dirty');
  assert.strictEqual(asked(entry({ session: { ...merged().s, unpushed: 2 } }), new Set(['clean:b1/s1'])).ask, 'dirty');
});

test('prune: a kept key goes once the document carries it or the session is gone, and stays while the document says nothing yet', () => {
  const doc = { repos: [{ rows: [{ board: 'b1', session: { id: 's1' } }, { board: 'b1', session: { id: 's2', cleanupKeptAt: 5 } }] }] };
  const kept = new Set(['clean:b1/s1', 'clean:b1/s2', 'clean:b1/s3', 'clean:b2/s1']);
  run('workAskPrune(kept, doc)', { kept, doc });
  assert.deepStrictEqual([...kept], ['clean:b1/s1']);
  run('workAskPrune(kept, null)', { kept, doc: null });
  assert.deepStrictEqual([...kept], ['clean:b1/s1']);
});

test('markup: the question has both buttons and both chips; the dirty line has no clean-up button and a way to the terminal; everything is escaped', () => {
  const a = { ask: 'clean', title: 'T', number: '12' };
  const html = run('workMergedAskHtml(a, o)', { a, o: { board: 'b"1', sid: 's<1', text: '<T>: ok' } });
  assert.ok(html.includes('data-wk-ask-act="cleanup"') && html.includes('>片付ける<') && html.includes('data-wk-ask-act="keep"') && html.includes('>残す<'));
  assert.ok(html.includes('未コミット 0') && html.includes('未 push 0') && html.includes('qc-chip ok') && html.includes('cleaning_services'));
  assert.ok(html.includes('&lt;T&gt;: ok') && html.includes('data-wk-ask-board="b&quot;1"') && html.includes('data-wk-ask-sid="s&lt;1"'));
  assert.ok(html.includes('btn-m3-primary') && html.includes('btn-m3-tonal') && html.split('<button type="button"').length === 3);
  assert.ok(!html.includes('disabled'));
  assert.ok(run('workMergedAskHtml(a, o)', { a, o: { board: 'b1', sid: 's1', busy: true } }).split(' disabled').length === 3);
  const dirty = run('workMergedAskHtml(a, o)', { a: { ask: 'dirty', files: 3, unpushed: 2 }, o: { board: 'b1', sid: 's1' } });
  assert.ok(dirty.includes('マージ済みですが、消えると困る変更があります（未 push 2 · 未コミット 3）') && dirty.includes('warning') && dirty.includes('aria-hidden'));
  assert.ok(dirty.includes('>ターミナルで確かめる<') && dirty.includes('data-wk-ask-act="check"') && dirty.includes('btn-m3-text'));
  assert.ok(!dirty.includes('cleanup') && !dirty.includes('>片付ける<') && !dirty.includes('qc-chip'));
});

const pageRow = (extra = {}) => ({ key: 'b1/t1', board: 'b1', hubSlug: 'srv1', ...merged(), ...extra });

test('keep: posts to the session on its own board, hides the question at once, and the failure is a note that leaves it', async () => {
  calls.length = 0;
  run('workAsk.kept.clear(); workAsk.busy.clear()');
  ctx.pageRows = [pageRow()];
  reply = { kept: true };
  await run('workAskPress("keep", "srv1", "s1")');
  assert.deepStrictEqual(plain(calls.filter(c => c[0] === 'api')), [['api', '/b/b1', '/api/sessions/s1/keep', 'POST', '{}']]);
  assert.deepStrictEqual([...run('workAsk.kept')], ['clean:srv1/s1']);
  assert.strictEqual(run('workAsk.busy.size'), 0);
  calls.length = 0;
  run('workAsk.kept.clear()');
  reply = new Error('no such session: s1');
  await run('workAskPress("keep", "srv1", "s1")');
  assert.ok(calls.some(c => c[0] === 'note' && c[1] === true));
  assert.strictEqual(run('workAsk.kept.size'), 0);
  assert.strictEqual(run('workAsk.busy.size'), 0);
});

test('press: the row is found by the board and the session id only; a session that is gone is a note; a press in flight blocks a second', async () => {
  calls.length = 0;
  run('workAsk.kept.clear(); workAsk.busy.clear()');
  ctx.pageRows = [pageRow({ hubSlug: 'srv2', board: 'b2', s: { ...merged().s, id: 's2' } }), pageRow({ s: { ...merged().s, id: 's2' } })];
  reply = { kept: true };
  await run('workAskPress("keep", "srv1", "s2")');
  assert.deepStrictEqual(plain(calls.filter(c => c[0] === 'api')), [['api', '/b/b1', '/api/sessions/s2/keep', 'POST', '{}']]);
  calls.length = 0;
  await run('workAskPress("keep", "srv1", "gone")');
  assert.deepStrictEqual(plain(calls.map(c => c[0])), ['note']);
  calls.length = 0;
  run('workAsk.busy.add("clean:srv1/s2")');
  await run('workAskPress("keep", "srv1", "s2")');
  assert.strictEqual(calls.length, 0);
  run('workAsk.busy.clear()');
});

test('press: 「片付ける」 from the document\'s facts posts the same clean-up as the row, with no read of the row\'s own; a dirty one posts nothing', async () => {
  calls.length = 0;
  run('workClean.git.clear(); workClean.busy.clear(); workAsk.busy.clear()');
  reply = { removed: true };
  ctx.pageRows = [pageRow({ selected: true })];
  await run('workAskPress("cleanup", "srv1", "s1")');
  assert.deepStrictEqual(plain(calls.filter(c => c[0] === 'api')), [['api', '/b/b1', '/api/sessions/s1/cleanup', 'POST', '{}']]);
  assert.ok(calls.some(c => c[0] === 'go') && calls.some(c => c[0] === 'refreshWork'));
  calls.length = 0;
  ctx.pageRows = [pageRow({ s: { ...merged().s, unpushed: 1 } })];
  await run('workAskPress("cleanup", "srv1", "s1")');
  assert.strictEqual(calls.filter(c => c[0] === 'api').length, 0);
});

test('press: a clean-up the server refuses marks the row blocked and opens the dialog; the row\'s busy set stops a second press from any button', async () => {
  calls.length = 0;
  run('workClean.git.clear(); workClean.busy.clear(); workAsk.busy.clear()');
  reply = { removed: false, git: clean, reasons: [{ detail: 'x' }] };
  ctx.pageRows = [pageRow()];
  await run('workAskPress("cleanup", "srv1", "s1")');
  assert.deepStrictEqual(plain(calls.find(c => c[0] === 'dialog')), ['dialog', 's1', '/b/b1', { blocked: true }]);
  assert.strictEqual(run('workClean.git.get("b1/s1").blocked'), true);
  calls.length = 0;
  run('workClean.busy.add("b1/s1")');
  await run('workCleanUp(r, { facts: true })', { r: pageRow() });
  assert.strictEqual(calls.length, 0);
  run('workClean.busy.clear(); workClean.git.clear()');
});
