/* The page's keeping of a PR's drafts (src/ui/pr-panel.js) against a fake `boardApi`, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'pr-panel.js'), 'utf8');
const viewSrc = fs.readFileSync(path.join(__dirname, '..', 'task-read-view.js'), 'utf8');
const readSrc = fs.readFileSync(path.join(__dirname, '..', 'pr-read.js'), 'utf8');
const cut = re => { const m = src.match(re); assert.ok(m, `not found: ${re}`); return m[0]; };
const fn = name => cut(new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));
// A function of pr-read.js, the pure half the page's drafts are made of.
const readFn = name => { const m = readSrc.match(new RegExp(`^function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm')); assert.ok(m, `not found: ${name}`); return m[0]; };

function world({ stored = null } = {}) {
  const w = { notes: [], redrawn: 0, posts: [], gets: 0, stored, gate: null, selected: 'pr:acme/widget#1', order: [], pends: [], refreshed: 0, pendGate: null, pendFail: null, pendRes: { sent: [], failed: [], skipped: [] } };
  const c = vm.createContext({
    structuredClone, wk: () => null, CSS: { escape: x => x }, PR_BOX_SEL: '.pr-draft-body, .pr-overall', JSON, Promise, clearTimeout, setTimeout: () => 1, console,
    get selectedTaskId() { return w.selected; }, PR_REF: 'pr:', PR_SAVE_MS: 800,
    note: (...a) => { w.notes.push(a); }, prDraftsChanged: () => { w.redrawn++; }, document: { getElementById: () => null },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => ({ id: 'acme/widget#1' }), prReadOf: () => c.read,
    prNextKey: (doc, prefix) => { let n = 1; while (doc.drafts.some(d => d.key === `${prefix}${n}`)) n++; return `${prefix}${n}`; },
    // What the page is on, as pr-panel.js says it: the selected review request.
    prShown: () => (typeof w.selected === 'string' && w.selected.startsWith('pr:') ? { id: w.selected.slice(3), rec: { id: w.selected.slice(3) }, kind: 'pr' } : null),
    refreshOthers: async () => { w.refreshed++; }, tp: () => null, cardDialogEl: () => null, renderTaskPanel() {}, prRedrawFor() {},
    boardApi: async (base, url, o) => {
      if (url === '/api/others/pending') {
        w.order.push('pend'); w.pends.push(JSON.parse(o.body));
        if (w.pendGate) await w.pendGate.promise;
        if (w.pendFail) throw new Error(w.pendFail);
        return w.pendRes;
      }
      if (!o) { w.gets++; if (!w.stored) throw Object.assign(new Error('no drafts'), { status: 404 }); return w.stored; }
      w.posts.push(JSON.parse(o.body)); w.order.push('save');
      if (w.gate) await w.gate.promise;
      return {};
    },
  });
  vm.runInContext(['prView', 'prDraftsEntry', 'prDocOfStored', 'prEnsureDrafts', 'prEditDrafts', 'prEditRefused', 'prScheduleSave', 'prSave', 'prShowSave', 'prSaveState', 'prInput', 'prFocusOut', 'prSettleOverall', 'prSaveSettled', 'prPend'].map(n => {
    return n === 'prView' ? `const prView = { drafts: new Map() };` : n === 'prSaveState' ? cut(/^const prSaveState = [^\n]*;/m) : fn(n);
  }).concat([cut(/^const PR_DRAFT_KEYS = [^\n]*;/m), fn('prSaveBody'), cut(/^const prDoc = [^\n]*;/m), readFn('prSeedDrafts'), readFn('prStampDoc'), readFn('prOverallShown')]).join('\n'), c);
  const run = e => vm.runInContext(e, c);
  return { w, c, run };
}
const tick = () => new Promise(r => setImmediate(r));
const read = { sha: 'a'.repeat(40), drafts: [{ path: 'a.rs', line: 1, side: 'right', body: 'seed' }] };
// A file with one comment in it, for the tests that edit one.
const seeded = () => ({ id: 'acme/widget#1', savedAt: '20261011T000000Z', version: 1, fromSha: read.sha, drafts: [{ key: 'p1', source: 'person', path: 'a.rs', line: 1, side: 'right', body: 'seed' }], overall: null });

test('no file yet: the first edit makes the file from an empty document, and saves it with the edit in it', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  // The read-through brings no comments of its own: even one of the older form, with drafts, seeds nothing.
  assert.deepStrictEqual(JSON.parse(JSON.stringify(run(`prDoc('acme/widget#1', read)`))), { version: 1, fromSha: read.sha, drafts: [], overall: null });
  assert.strictEqual(run(`prEditDrafts('acme/widget#1', read, d => { d.drafts.push({ key: 'p1', source: 'person', path: 'a.rs', line: 1, side: 'right', body: 'edited', sha: read.sha }); prStampDoc(d, read); })`), true);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts.length, 1);
  assert.strictEqual(w.posts[0].id, 'acme/widget#1');
  // The head it was written on is a top-level map, never a field of the draft (an older binary refuses unknown fields there).
  assert.deepStrictEqual(w.posts[0].drafts.drafts, [{ key: 'p1', source: 'person', path: 'a.rs', line: 1, side: 'right', body: 'edited' }]);
  assert.deepStrictEqual(w.posts[0].drafts.shas, { p1: read.sha });
  assert.strictEqual(w.posts[0].drafts.fromSha, read.sha);
  assert.strictEqual(w.posts[0].drafts.overall, null);
});

test('a read-through seeds nothing: an empty document with the head and the mode it is on', () => {
  const { run, c } = world();
  c.old = { sha: 'b'.repeat(40), drafts: [{ path: 'a.rs', line: 1, body: 'x' }], overall: { body: 'summary', suggestion: 'approve' } };
  assert.deepStrictEqual(JSON.parse(JSON.stringify(run('prSeedDrafts(old)'))), { version: 1, fromSha: 'b'.repeat(40), drafts: [], overall: null });
  c.split = { sha: 'c'.repeat(40), mode: 'split', overall: { body: 'plan' } };
  assert.deepStrictEqual(JSON.parse(JSON.stringify(run('prSeedDrafts(split)'))), { version: 1, fromSha: 'c'.repeat(40), fromMode: 'split', drafts: [], overall: null });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(run('prSeedDrafts(null)'))), { version: 1, fromSha: null, drafts: [], overall: null });
  // Every edit that writes a comment says which head it was written on, and the mode (the older one is not a split).
  const doc = { fromSha: 'a'.repeat(40), fromMode: 'split', drafts: [] };
  c.doc = doc;
  run('prStampDoc(doc, old)');
  assert.deepStrictEqual(JSON.parse(JSON.stringify(doc)), { fromSha: 'b'.repeat(40), drafts: [] });
  run('prStampDoc(doc, split)');
  assert.deepStrictEqual(JSON.parse(JSON.stringify(doc)), { fromSha: 'c'.repeat(40), fromMode: 'split', drafts: [] });
  run('prStampDoc(doc, null)');
  assert.strictEqual(doc.fromSha, 'c'.repeat(40));
});

test('an edit made while a save is in flight is saved afterwards', async () => {
  const { w, c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  let release; w.gate = { promise: new Promise(r => { release = r; }) };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'one'; })`);
  const first = run(`prSave('acme/widget#1')`);
  await tick();
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'two'; })`);
  w.gate = null; release();
  await first; await tick(); await tick();
  assert.deepStrictEqual(w.posts.map(p => p.drafts.drafts[0].body), ['one', 'two']);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), false);
});

test('a failed save keeps the edit, and says so once when it starts to fail', async () => {
  const { w, c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw new Error('boom'); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'keep'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), true);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'boom');
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'keep');
  // The comments are on the lines, where no save line is: the failure is a note, once.
  assert.deepStrictEqual(w.notes, [['コメントの保存', true, 'boom 入力はこのページに残っています']]);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.notes.length, 1);
});

test('the save body has only the known keys, even when the stored document had another', async () => {
  const stored = { id: 'acme/widget#1', savedAt: '20261011T000000Z', version: 1, fromSha: read.sha, pending: { x: 1 },
    drafts: [{ key: 'p1', source: 'person', path: 'a.rs', line: 2, body: 'b', extra: 1 }], shas: { p1: read.sha, gone: read.sha }, overall: { body: 'o', more: 1 } };
  const { w, c, run } = world({ stored });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'c'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts).sort(), ['drafts', 'fromSha', 'overall', 'shas', 'version']);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts.drafts[0]).sort(), ['body', 'key', 'line', 'path', 'source']);
  // Read back from the server, the map is put on the drafts again; a draft deleted drops its entry; no map when none has a head.
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].sha`), read.sha);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).shas`), undefined);
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts = []; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts[1].drafts.shas, undefined);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts.overall), ['body']);
});

test('the file is not asked for again while an unsaved edit is held', async () => {
  const { w, c, run } = world({ stored: seeded() });
  Object.assign(c, { prEnsureRead() {} });
  run(fn('prEnter'));
  run(`prView.shownFor = null;`);
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'unsaved'; })`);
  // A new selection reloads the drafts only when they are clean.
  run(`prEnter('acme/widget#1', { id: 'acme/widget#1' })`);
  await tick();
  assert.strictEqual(w.gets, 1);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'unsaved');
  // Clean, and entered again as another selection: asked for again.
  await run(`prSave('acme/widget#1')`);
  run(`prView.shownFor = null; prEnter('acme/widget#1', { id: 'acme/widget#1' })`);
  await tick();
  assert.strictEqual(w.gets, 2);
});

const overallBox = value => ({ value, dataset: {}, matches: sel => sel.split(',').some(x => x.trim() === '.pr-overall') });

test('typing in the overall comment edits the document and schedules a save, and what else is typed in is not a box of the drafts', async () => {
  const { w, c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.input = { target: overallBox('my summary') };
  run('prInput(input)');
  assert.deepStrictEqual(JSON.parse(JSON.stringify(run(`prDoc('acme/widget#1', read).overall`))), { body: 'my summary' });
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), true);
  // A textarea of a comment on a line is the diff's own, typed in without a document.
  const comment = { value: 'x', dataset: { prdBox: 'p1' }, matches: () => false };
  c.input = { target: comment };
  run('prInput(input)');
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'seed');
  assert.strictEqual(w.notes.length, 0);
  // Leaving the box saves at once.
  c.out = { target: overallBox('my summary') };
  run('prFocusOut(out)');
  await tick(); await tick();
  assert.strictEqual(w.posts.length, 1);
  assert.strictEqual(w.posts[0].drafts.overall.body, 'my summary');
});

test('a refused save shows the server\'s problems with the message', async () => {
  const { c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw Object.assign(new Error('the drafts are not valid'), { problems: ['drafts[0].key: bad', 'overall: too long'] }); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'x'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'the drafts are not valid (drafts[0].key: bad / overall: too long)');
});

function miscWorld() {
  const c = vm.createContext({});
  vm.runInContext(['const prView = { last: null, shownFor: "acme/widget#1" };', fn('prLeft')].join('\n'), c);
  return { c, run: e => vm.runInContext(e, c) };
}

test('the PR shown last is remembered in memory, and only a new selection sets it', async () => {
  const { c, run } = world();
  Object.assign(c, { prEnsureRead() {} });
  run(fn('prEnter'));
  run('prView.shownFor = null; prView.last = null;');
  run(`prEnter('acme/widget#1', { id: 'acme/widget#1' })`);
  assert.strictEqual(run('prView.last'), 'acme/widget#1');
  run(`prEnter('acme/widget#2', null)`);
  assert.strictEqual(run('prView.last'), 'acme/widget#2');
  // Leaving the PR makes the next showing a new selection but does not forget which one was shown last.
  run('prView.shownFor = null');
  assert.strictEqual(run('prView.last'), 'acme/widget#2');
});

test('only three of the server\'s problems are shown, and how many more there are', async () => {
  const { c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw Object.assign(new Error('bad'), { problems: ['a', 'b', 'c', 'd', 'e'] }); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'x'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'bad (a / b / c / 他 2 件)');
});

test('leaving the PR makes the next showing of it a new selection', () => {
  const { run } = miscWorld();
  run('prLeft()');
  assert.strictEqual(run('prView.shownFor'), null);
});

test('typing in a box while nothing is on show is ignored', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.selected = 'pr:acme/widget#1';
  w.selected = 'work:somewhere';
  c.input = { target: overallBox('x') };
  run('prInput(input)');
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), false);
  assert.strictEqual(w.notes.length, 0);
});

/* ── putting the drafts into the pending review ── */

const withSent = { id: 'acme/widget#1', savedAt: '20261011T000000Z', version: 1, fromSha: read.sha,
  drafts: [{ key: 'r1', source: 'reader', path: 'a.rs', line: 1, side: 'right', body: 'seed', sentAt: '20261011T000100Z' }],
  overall: { body: '' }, pending: { reviewId: 'PRR_1', url: 'https://github.com/acme/widget/pull/1#pullrequestreview-1', at: '20261011T000100Z' } };

test('a push saves what is unsaved first, and makes the file with the overall comment the dialog shows before it', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = { ...read, overall: { body: 'the reader\'s summary' } };
  await run(`prPend('acme/widget#1', read)`);
  // No file yet: the file was made with what is on screen in it, then pushed.
  assert.deepStrictEqual(w.order, ['save', 'pend']);
  assert.deepStrictEqual(w.posts[0].drafts.overall, { body: 'the reader\'s summary' });
  assert.deepStrictEqual(w.pends, [{ id: 'acme/widget#1' }]);

  const b = world({ stored: seeded() });
  b.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  b.c.read = read;
  b.run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'typed'; })`);
  await b.run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(b.w.order, ['save', 'pend']);
  assert.strictEqual(b.w.posts[0].drafts.drafts[0].body, 'typed');
});

test('the overall comment shown is the stored one when the document has one, even blank, else the reader\'s; a push writes the shown one into the document first', async () => {
  // The reader's own text is shown while the document has no overall comment, and it is the reader's untouched one.
  const a = world({ stored: seeded() });
  a.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  a.c.read = { ...read, overall: { body: 'summary' } };
  assert.deepStrictEqual(JSON.parse(JSON.stringify(a.run(`prOverallShown(prDoc('acme/widget#1', read), read)`))), { text: 'summary', reader: true });
  // Blank reader text: nothing to show, nothing the reader said.
  assert.deepStrictEqual(JSON.parse(JSON.stringify(a.run(`prOverallShown(prDoc('acme/widget#1', read), { overall: { body: ' ' } })`))), { text: ' ', reader: false });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(a.run(`prOverallShown(prDoc('acme/widget#1', read), null)`))), { text: '', reader: false });
  // What the push sends is what was shown.
  await a.run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(a.w.posts[0].drafts.overall, { body: 'summary' });
  // The person's own, even blank, wins: the reader's text does not come back.
  const b = world({ stored: { ...seeded(), overall: { body: '' } } });
  b.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  b.c.read = { ...read, overall: { body: 'summary' } };
  assert.deepStrictEqual(JSON.parse(JSON.stringify(b.run(`prOverallShown(prDoc('acme/widget#1', read), read)`))), { text: '', reader: false });
  await b.run(`prPend('acme/widget#1', read)`);
  assert.strictEqual(b.w.posts.length, 0, 'nothing was written, so nothing is saved');
  assert.deepStrictEqual(JSON.parse(JSON.stringify(b.run(`prDoc('acme/widget#1', read).overall`))), { body: '' });
  // Nothing to write when the reader had nothing: the file has no overall comment.
  const c2 = world({ stored: seeded() });
  c2.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c2.c.read = { ...read, overall: null };
  await c2.run(`prPend('acme/widget#1', read)`);
  assert.strictEqual(c2.w.posts.length, 0);
  assert.strictEqual(c2.run(`prDoc('acme/widget#1', read).overall`), null);
  // A hand-over does the same through `prSettleOverall`, which is a no-op once the document has one.
  const d = world({ stored: seeded() });
  d.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  d.c.read = { ...read, overall: { body: 'summary' } };
  d.run(`prSettleOverall('acme/widget#1', read)`);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(d.run(`prDoc('acme/widget#1', read).overall`))), { body: 'summary' });
  d.run(`prEditDrafts('acme/widget#1', read, d => { d.overall = { body: 'changed' }; })`);
  d.run(`prSettleOverall('acme/widget#1', read)`);
  assert.strictEqual(d.run(`prDoc('acme/widget#1', read).overall.body`), 'changed');
});

test('a push waits for the save that is already in flight', async () => {
  const { w, c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  let release; w.gate = { promise: new Promise(r => { release = r; }) };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'one'; })`);
  run(`prSave('acme/widget#1')`);
  await tick();
  const pend = run(`prPend('acme/widget#1', read)`);
  await tick();
  assert.deepStrictEqual(w.pends, []);
  w.gate = null; release();
  await pend;
  assert.deepStrictEqual(w.order, ['save', 'pend']);
});

test('a push that cannot save first does not push, and keeps the drafts', async () => {
  const { w, c, run } = world({ stored: seeded() });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async (b, url) => { if (url === '/api/others/pending') w.order.push('pend'); throw new Error('boom'); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'keep'; })`);
  await run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(w.order, []);
  assert.ok(w.notes.at(-1)[2].includes('boom'));
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').pending`), false);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'keep');
});

test('a second click while a push is in flight does not send again, and the drafts cannot be edited meanwhile', async () => {
  const { w, c, run } = world({ stored: withSent });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  let release; w.pendGate = { promise: new Promise(r => { release = r; }) };
  const first = run(`prPend('acme/widget#1', read)`);
  await tick();
  await run(`prPend('acme/widget#1', read)`);
  assert.strictEqual(w.pends.length, 1);
  assert.strictEqual(run(`prEditDrafts('acme/widget#1', read, d => { d.overall = { body: 'x' }; })`), false);
  w.pendGate = null; release();
  await first;
  assert.strictEqual(w.pends.length, 1);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').pending`), false);
});

test('a push that went through takes the server\'s document, with the marks, and reads the records again', async () => {
  const stored = { ...withSent, drafts: [{ ...withSent.drafts[0], sentAt: undefined }], pending: undefined };
  const { w, c, run } = world({ stored });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  w.pendRes = { drafts: withSent, sent: ['r1'], failed: [{ key: 'r2', why: 'Line must be part of the diff' }], skipped: [], overall: 'none', warning: null };
  await run(`prPend('acme/widget#1', read)`);
  const e = JSON.parse(JSON.stringify(run(`prDraftsEntry('acme/widget#1')`)));
  assert.strictEqual(e.doc.drafts[0].sentAt, '20261011T000100Z');
  assert.strictEqual(e.doc.pending.reviewId, 'PRR_1');
  assert.strictEqual(e.doc.id, undefined);
  assert.strictEqual(e.dirty, false);
  assert.deepStrictEqual(e.pendResult, { sent: 1, failed: [{ key: 'r2', why: 'Line must be part of the diff' }], skipped: [], overall: 'none' });
  assert.strictEqual(w.refreshed, 1);
});

test('a push that failed keeps the document and says why', async () => {
  const { w, c, run } = world({ stored: { ...withSent, drafts: [{ ...withSent.drafts[0], sentAt: undefined }] } });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  w.pendFail = 'GitHub did not take the drafts: nope';
  await run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(w.notes.at(-1), ['pending review', true, 'GitHub did not take the drafts: nope']);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].sentAt`), undefined);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').pendResult`), null);
  assert.strictEqual(w.refreshed, 0);
});

test('a save sends the marks back', async () => {
  const { w, c, run } = world({ stored: withSent });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.overall = { body: 'o' }; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts[0].drafts.drafts[0].sentAt, '20261011T000100Z');
  assert.strictEqual(w.posts[0].drafts.pending, undefined);
});

test('when adj could not write its file after GitHub took the drafts, they are marked here and are not sent again', async () => {
  const stored = { ...withSent, drafts: [{ ...withSent.drafts[0], sentAt: undefined }, { key: 'r2', source: 'person', path: 'b.rs', line: 2, side: 'right', body: 'refused' }], pending: undefined };
  const { w, c, run } = world({ stored });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  w.pendRes = { drafts: null, sent: ['r1'], failed: [{ key: 'r2', why: 'no' }], skipped: [], overall: 'none', warning: 'cannot write' };
  await run(`prPend('acme/widget#1', read)`);
  const docs = JSON.parse(JSON.stringify(run(`prDoc('acme/widget#1', read).drafts`)));
  assert.match(docs[0].sentAt, /^\d{8}T\d{6}Z$/);
  assert.strictEqual(docs[1].sentAt, undefined);
  assert.ok(w.notes.some(n => n[2].includes('cannot write')));
  // Saved with the mark, so the file learns it too.
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts.at(-1).drafts.drafts[0].sentAt, docs[0].sentAt);
});

test('an error of the records being read again does not turn a push that went through into a failure', async () => {
  const { w, c, run } = world({ stored: { ...withSent, drafts: [{ ...withSent.drafts[0], sentAt: undefined }] } });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.refreshOthers = async () => { throw new Error('records'); };
  w.pendRes = { drafts: withSent, sent: ['r1'], failed: [], skipped: [], overall: 'none', warning: null };
  await run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(w.notes, []);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].sentAt`), '20261011T000100Z');
});

test('what the last push said about refused drafts goes when the drafts are edited', async () => {
  const { w, c, run } = world({ stored: { ...withSent, drafts: [{ ...withSent.drafts[0], sentAt: undefined }] } });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  w.pendRes = { drafts: withSent, sent: [], failed: [{ key: 'r1', why: 'no' }], skipped: [], overall: 'none', warning: null };
  await run(`prPend('acme/widget#1', read)`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').pendResult.failed.length`), 1);
  run(`prEditDrafts('acme/widget#1', read, d => { d.overall = { body: 'x' }; })`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').pendResult`), null);
});

/* What a click does for the buttons that belong to a PR (`prAct`), against the page's drafts. The pure parts are pr-read.js's, loaded with it. */
function offerWorld(doc, read, rec = { id: 'acme/widget#1', url: 'u' }) {
  const w = { redrawn: [], edits: 0 };
  const c = vm.createContext({
    CSS: { escape: x => x }, structuredClone, JSON, console, selectedTaskId: 'pr:acme/widget#1', PR_REF: 'pr:', PR_HOOKS: { diff: null },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => rec, OTHERS_LIVE: ['requested', 'ai-reading', 'ai-ready', 'pushed'],
    prReadOf: () => read, prDraftsChanged: id => { w.redrawn.push(id); }, prScheduleSave() {}, prShowSave() {}, note() {}, othersRequestAi() {}, cardDialog: {},
    // What the page is on, as pr-panel.js says it: the selected review request, asked through the list's endpoint.
    prShown: () => ({ id: 'acme/widget#1', rec, kind: 'pr', ask: mode => c.othersRequestAi('acme/widget#1', mode) }),
    secTitle: t => t, panelBtnsHtml: () => '',
  });
  const files = ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'];
  vm.runInContext(files.map(f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8')).join('\n'), c);
  vm.runInContext([
    'const prView = { drafts: new Map() };', fn('prDraftsEntry'), fn('prEditDrafts'), cut(/^const prDoc = [^\n]*;/m), cut(/^const prSaveState = [^\n]*;/m), fn('prAct'),
  ].join('\n'), c);
  const e = vm.runInContext(`prDraftsEntry('acme/widget#1')`, c);
  Object.assign(e, { doc, saved: true, loaded: true });
  return { w, c, e };
}
test('a split proposal is the overall comment the dialog shows until the person types in it; the re-read notice is gone', () => {
  const proposal = { sha: 'b'.repeat(40), mode: 'split', drafts: [], overall: { body: 'split plan' } };
  const doc = { version: 1, fromSha: 'a'.repeat(40), drafts: [{ key: 'p1', source: 'person', path: 'a.rs', body: 'mine' }], overall: { body: 'my note' } };
  const { c } = offerWorld(doc, proposal);
  const shown = d => JSON.parse(JSON.stringify(vm.runInContext(`prOverallShown(${JSON.stringify(d)}, ${JSON.stringify(proposal)})`, c)));
  // The person's own note stays, and the proposal is not put after it or over it.
  assert.deepStrictEqual(shown(doc), { text: 'my note', reader: false });
  assert.deepStrictEqual(shown({ ...doc, overall: { body: '' } }), { text: '', reader: false });
  assert.deepStrictEqual(shown({ ...doc, overall: null }), { text: 'split plan', reader: true });
  // Nothing of the older notice is left to be clicked or drawn.
  for (const gone of ['prNewReaderDrafts', 'prNewReaderOverall', 'prFromElsewhere', 'prDraftsBodyHtml', 'prDraftsHtml']) assert.strictEqual(vm.runInContext(`typeof ${gone}`, c), 'undefined', gone);
  assert.ok(!src.includes('data-pr-take-new') && !src.includes('data-pr-keep'));
});

test('only the split button asks for a proposal; the re-read button asks for a read, whatever is stored', () => {
  const asked = [];
  const bigRec = { id: 'acme/widget#1', url: 'u', state: 'ai-ready', files: [{ path: 'b.rs', additions: 2000, deletions: 0, change: 'modified' }] };
  const click = (sel, read, rec = bigRec) => {
    const { c } = offerWorld({ version: 1, fromSha: null, drafts: [], overall: { body: '' } }, read, rec);
    c.othersRequestAi = (id, mode) => asked.push([id, mode]);
    // The button the click is on: it matches the selector it was found by, and nothing else.
    c.button = { matches: s => s.split(',').some(x => x.trim() === sel), disabled: false, getAttribute: () => null };
    c.sel = sel;
    vm.runInContext('prAct({ target: { closest: s => s.includes(sel) ? button : null } })', c);
  };
  click('[data-pr-split]', null);
  click('[data-pr-ai]', null);
  click('[data-pr-reread]', { mode: 'split', sha: 'a'.repeat(40) });
  click('[data-pr-reread]', { sha: 'a'.repeat(40) });
  // Small now: the same.
  click('[data-pr-reread]', { mode: 'split', sha: 'a'.repeat(40) }, { ...bigRec, files: [] });
  assert.deepStrictEqual(asked, [['acme/widget#1', 'split'], ['acme/widget#1', undefined], ['acme/widget#1', undefined], ['acme/widget#1', undefined], ['acme/widget#1', undefined]]);
});

test('the reader\'s place is drawn with the stored read-through, so a PR out of review with a stale one is offered its buttons', () => {
  const rec = { id: 'acme/widget#1', repo: 'a/w', number: 1, state: 'ai-ready', headSha: 'b'.repeat(40),
    files: [{ path: 'src/big.rs', additions: 2000, deletions: 0, change: 'modified' }], ai: { status: 'ready', sha: 'a'.repeat(40), stale: true } };
  let read = { sha: 'a'.repeat(40), three: {}, overall: null };
  let readError = null;
  const drawn = [];
  const panelTerm = { term: null, ended: null };
  const c = vm.createContext({
    esc: x => String(x), others: { doc: {}, error: null, asking: new Set() }, panelTerm,
    prReadOf: () => read, prReadEntry: () => ({ error: readError }), othersNowEst: () => 0, console,
    syncReaderTerm() {}, prReaderSubject: () => 's', prTermEnd: () => ({}), tp: () => ({ hidden: true }), setPanelPart: (name, el, html) => { drawn.push([name, html]); },
  });
  const files = ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'];
  vm.runInContext(files.map(f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8')).join('\n'), c);
  const draw = () => {
    drawn.length = 0;
    vm.runInContext(viewSrc.match(/^function drawReaderTerm\([^)]*\) \{[\s\S]*?\n\}/m)[0] + '\ndrawReaderTerm(rec, [rec], { canRead: true, why: "" }, "missing");', c);
    return drawn.find(([name]) => name === 'ph')[1];
  };
  c.rec = rec;
  const html = draw();
  assert.ok(html.includes('data-pr-split') && html.includes('src/big.rs'), html);
  // Still being fetched: nothing is offered yet.
  read = undefined;
  assert.ok(!draw().includes('data-pr-split'));
  // Fetching it keeps failing: the buttons come back, since a request does not depend on it.
  readError = 'boom';
  assert.ok(draw().includes('data-pr-split'));
  // No record: what the caller says. A live terminal: nothing stands over it.
  c.rec = null;
  assert.strictEqual(draw(), 'missing');
  c.rec = rec;
  panelTerm.term = {};
  assert.strictEqual(draw(), '');
});

test('a PR that is out of review but waits for nothing keeps its status line', () => {
  const rec = { id: 'acme/widget#1', repo: 'a/w', number: 1, state: 'waiting-on-author', base: 'main', head: 'f', headSha: 'a'.repeat(40), ai: null,
    files: [{ path: 'src/big.rs', additions: 2000, deletions: 0, change: 'modified' }] };
  const c = vm.createContext({ esc: x => String(x), secTitle: t => t, panelBtnsHtml: () => '', console });
  vm.runInContext(['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'].map(f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8')).join('\n'), c);
  c.rec = rec;
  const html = vm.runInContext('prDetailHtml(rec, null, [rec], { canRead: true, why: "" }, 0)', c);
  assert.ok(html.includes('data-pr-sec="top:ai"') && !html.includes('top:oor'));
});
