/* The page's keeping of a PR's drafts (src/ui/pr-panel.js) against a fake `boardApi`, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'pr-panel.js'), 'utf8');
const viewSrc = fs.readFileSync(path.join(__dirname, '..', 'task-read-view.js'), 'utf8');
const cut = re => { const m = src.match(re); assert.ok(m, `not found: ${re}`); return m[0]; };
const fn = name => cut(new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

function world({ stored = null } = {}) {
  const w = { notes: [], redrawn: 0, posts: [], gets: 0, stored, gate: null, selected: 'pr:acme/widget#1', order: [], pends: [], refreshed: 0, pendGate: null, pendFail: null, pendRes: { sent: [], failed: [], skipped: [] } };
  const c = vm.createContext({
    structuredClone, wk: () => null, CSS: { escape: x => x }, PR_BOX_SEL: '.pr-draft-body, .pr-overall', JSON, Promise, clearTimeout, setTimeout: () => 1, console,
    get selectedTaskId() { return w.selected; }, PR_REF: 'pr:', PR_SAVE_MS: 800,
    note: (...a) => { w.notes.push(a); }, prRedrawNow: () => { w.redrawn++; },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => ({ id: 'acme/widget#1' }), prReadOf: () => c.read,
    prNextKey: (doc, prefix) => { let n = 1; while (doc.drafts.some(d => d.key === `${prefix}${n}`)) n++; return `${prefix}${n}`; },
    // What the page is on, as pr-panel.js says it: the selected review request.
    prShown: () => (typeof w.selected === 'string' && w.selected.startsWith('pr:') ? { id: w.selected.slice(3), rec: { id: w.selected.slice(3) }, kind: 'pr' } : null),
    refreshOthers: async () => { w.refreshed++; }, tp: () => null, cardDialogEl: () => null, renderTaskPanel() {}, prRedrawFor() {},
    prSeedDrafts: read => ({ version: 1, fromSha: read?.sha ?? null, drafts: (read?.drafts || []).map((d, i) => ({ key: `r${i + 1}`, source: 'reader', ...d })), overall: { body: '' } }),
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
  vm.runInContext(['prView', 'prDraftsEntry', 'prDocOfStored', 'prEnsureDrafts', 'prEditDrafts', 'prScheduleSave', 'prSave', 'prShowSave', 'prSaveState', 'prInput', 'prSaveSettled', 'prPend'].map(n => {
    return n === 'prView' ? `const prView = { drafts: new Map() };` : n === 'prSaveState' ? cut(/^const prSaveState = [^\n]*;/m) : fn(n);
  }).concat([cut(/^const PR_DRAFT_KEYS = [^\n]*;/m), fn('prSaveBody'), cut(/^const prDoc = [^\n]*;/m)]).join('\n'), c);
  const run = e => vm.runInContext(e, c);
  return { w, c, run };
}
const tick = () => new Promise(r => setImmediate(r));
const read = { sha: 'a'.repeat(40), drafts: [{ path: 'a.rs', line: 1, side: 'right', body: 'seed' }] };

test('no file yet: the first edit saves the seed with the edit in it', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  assert.strictEqual(run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'edited'; })`), true);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts.length, 1);
  assert.strictEqual(w.posts[0].id, 'acme/widget#1');
  assert.strictEqual(w.posts[0].drafts.drafts[0].body, 'edited');
  assert.strictEqual(w.posts[0].drafts.fromSha, read.sha);
});

test('an edit made while a save is in flight is saved afterwards', async () => {
  const { w, c, run } = world();
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

test('a failed save keeps the edit and says so', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw new Error('boom'); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'keep'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), true);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'boom');
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'keep');
});

test('the save body has only the known keys, even when the stored document had another', async () => {
  const stored = { id: 'acme/widget#1', savedAt: '20261011T000000Z', version: 1, fromSha: read.sha, pending: { x: 1 },
    drafts: [{ key: 'p1', source: 'person', path: 'a.rs', line: 2, body: 'b', extra: 1 }], overall: { body: 'o', more: 1 } };
  const { w, c, run } = world({ stored });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'c'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts).sort(), ['drafts', 'fromSha', 'overall', 'version']);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts.drafts[0]).sort(), ['body', 'key', 'line', 'path', 'source']);
  assert.deepStrictEqual(Object.keys(w.posts[0].drafts.overall), ['body']);
});

test('the file is not asked for again while an unsaved edit is held', async () => {
  const { w, c, run } = world();
  Object.assign(c, { prefs: { workTab: 'others' }, prEnsureRead() {}, renderWorkTabs() {}, renderOthersList() {} });
  run(fn('prEnter'));
  run(`prView.shownFor = null; prView.tabWas = null;`);
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

test('typing into a box whose draft is gone keeps the text as a draft of its own', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts = []; })`);
  const box = { value: 'typed into a stale box', dataset: { prBody: 'r1' }, matches: sel => sel.includes('.pr-draft-body') && !sel.startsWith('.pr-overall') };
  run('0'); c.input = { target: box };
  run('prInput(input)');
  const drafts = run(`prDoc('acme/widget#1', read).drafts`);
  assert.strictEqual(drafts.length, 1);
  assert.strictEqual(drafts[0].body, 'typed into a stale box');
  assert.strictEqual(drafts[0].source, 'person');
  assert.strictEqual(w.notes.length, 1);
  assert.strictEqual(w.redrawn, 1);
});

test('a refused save shows the server\'s problems with the message', async () => {
  const { c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw Object.assign(new Error('the drafts are not valid'), { problems: ['drafts[0].key: bad', 'overall: too long'] }); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'x'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'the drafts are not valid (drafts[0].key: bad / overall: too long)');
});

function miscWorld(prefs) {
  const c = vm.createContext({ prefs });
  vm.runInContext(['const prView = { tabWas: null, focusNext: null, shownFor: "acme/widget#1" };', fn('prPrefsToSave'), fn('prKeepTab'), fn('prRestoreFocus'), fn('prLeft')].join('\n'), c);
  return { c, run: e => vm.runInContext(e, c) };
}

test('a PR link\'s tab is not what gets saved, and a deliberate choice of the tab ends that', () => {
  const prefs = { workTab: 'others', othersRepo: 'x' };
  const { run } = miscWorld(prefs);
  // The link flipped the tab from the saved 「タスク」.
  run(`prView.tabWas = 'task'`);
  assert.strictEqual(run('prPrefsToSave().workTab'), 'task');
  assert.strictEqual(run('prPrefsToSave().othersRepo'), 'x');
  // Choosing 「AI 下読み」 on purpose: it is what is saved from now on.
  assert.strictEqual(run('prKeepTab()'), true);
  assert.strictEqual(run('prPrefsToSave().workTab'), 'others');
  assert.strictEqual(run('prKeepTab()'), false);
  // Choosing 「タスク」 after a link also ends it.
  run(`prView.tabWas = 'task'; prefs.workTab = 'task'`);
  assert.strictEqual(run('prPrefsToSave().workTab'), 'task');
  assert.strictEqual(run('prView.tabWas'), null);
});

test('after a forced redraw the box an action asked for has focus, else the one that had it keeps its caret', () => {
  const { run, c } = miscWorld({ workTab: 'others' });
  const box = key => ({ key, focused: 0, range: null, matches: () => true, focus() { this.focused++; }, setSelectionRange(a, b) { this.range = [a, b]; } });
  const boxes = { '[data-pr-body="p2"]': box('p2'), '[data-pr-body="r1"]': box('r1') };
  c.root = { querySelector: sel => boxes[sel] || null };
  // An action asked for p2 (in the panel): it gets focus, and the old box's caret is not forced onto it.
  c.keep = { sel: '[data-pr-body="r1"]', a: 3, b: 5 };
  run(`prView.focusNext = { sel: '[data-pr-body="p2"]', inDialog: false }`);
  run('prRestoreFocus(root, keep, false)');
  assert.strictEqual(boxes['[data-pr-body="p2"]'].focused, 1);
  assert.strictEqual(boxes['[data-pr-body="p2"]'].range, null);
  // Asked for the dialog's, the panel's redraw leaves it alone and keeps the box that had focus, with its caret.
  run('prRestoreFocus(root, keep, true)');
  run(`prView.focusNext = { sel: '[data-pr-body="p2"]', inDialog: true }`);
  run('prRestoreFocus(root, keep, false)');
  assert.strictEqual(boxes['[data-pr-body="p2"]'].focused, 1);
  assert.deepStrictEqual(boxes['[data-pr-body="r1"]'].range, [3, 5]);
  // Nothing asked and the control is gone: nothing is focused.
  run('prView.focusNext = null');
  c.keep = { sel: '[data-pr-body="gone"]', a: 0, b: 0 };
  assert.strictEqual(run('prRestoreFocus(root, keep, false)'), null);
});

test('only three of the server\'s problems are shown, and how many more there are', async () => {
  const { c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.boardApi = async () => { throw Object.assign(new Error('bad'), { problems: ['a', 'b', 'c', 'd', 'e'] }); };
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'x'; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').saveError`), 'bad (a / b / c / 他 2 件)');
});

test('leaving the PR makes the next showing of it a new selection', () => {
  const { run } = miscWorld({ workTab: 'others' });
  run('prLeft()');
  assert.strictEqual(run('prView.shownFor'), null);
});

test('an input in a dialog about another subject is ignored', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  c.cardDialog = { subject: 'pr:other/repo#2' };
  const box = { value: 'x', dataset: { prBody: 'r1' }, matches: sel => sel.includes('.pr-draft-body'), closest: sel => sel === '#card-dialog' ? {} : null };
  c.input = { target: box };
  run('prInput(input)');
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), false);
  assert.strictEqual(w.notes.length, 0);
});

/* ── putting the drafts into the pending review ── */

const withSent = { id: 'acme/widget#1', savedAt: '20261011T000000Z', version: 1, fromSha: read.sha,
  drafts: [{ key: 'r1', source: 'reader', path: 'a.rs', line: 1, side: 'right', body: 'seed', sentAt: '20261011T000100Z' }],
  overall: { body: '' }, pending: { reviewId: 'PRR_1', url: 'https://github.com/acme/widget/pull/1#pullrequestreview-1', at: '20261011T000100Z' } };

test('a push saves what is unsaved first, and the reader\'s drafts become the file before it', async () => {
  const { w, c, run } = world();
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  await run(`prPend('acme/widget#1', read)`);
  // No file yet: the seed was saved as it stood, then pushed.
  assert.deepStrictEqual(w.order, ['save', 'pend']);
  assert.strictEqual(w.posts[0].drafts.drafts[0].body, 'seed');
  assert.deepStrictEqual(w.pends, [{ id: 'acme/widget#1' }]);

  const b = world();
  b.run(`prEnsureDrafts('acme/widget#1')`); await tick();
  b.c.read = read;
  b.run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'typed'; })`);
  await b.run(`prPend('acme/widget#1', read)`);
  assert.deepStrictEqual(b.w.order, ['save', 'pend']);
  assert.strictEqual(b.w.posts[0].drafts.drafts[0].body, 'typed');
});

test('a push waits for the save that is already in flight', async () => {
  const { w, c, run } = world();
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
  const { w, c, run } = world();
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

test('a save sends the marks back, and typing into a draft that was sent changes nothing', async () => {
  const { w, c, run } = world({ stored: withSent });
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.overall = { body: 'o' }; })`);
  await run(`prSave('acme/widget#1')`);
  assert.strictEqual(w.posts[0].drafts.drafts[0].sentAt, '20261011T000100Z');
  assert.strictEqual(w.posts[0].drafts.pending, undefined);

  const box = { value: 'edited anyway', dataset: { prBody: 'r1' }, matches: sel => sel.includes('.pr-draft-body') };
  c.input = { target: box };
  run('prInput(input)');
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'seed');
  assert.strictEqual(run(`prDraftsEntry('acme/widget#1').dirty`), false);
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

/* A split proposal after the person's file exists: the overall comment is offered with the new drafts, and added after what the
   person wrote. The pure parts are pr-read.js's, loaded with it. */
function offerWorld(doc, read, rec = { id: 'acme/widget#1', url: 'u' }) {
  const w = { redrawn: [], edits: 0 };
  const c = vm.createContext({
    CSS: { escape: x => x }, structuredClone, JSON, console, selectedTaskId: 'pr:acme/widget#1', PR_REF: 'pr:', PR_HOOKS: { diff: null },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => rec, OTHERS_LIVE: ['requested', 'ai-reading', 'ai-ready', 'pushed'],
    prReadOf: () => read, prRedrawNow: sel => { w.redrawn.push(sel); }, prScheduleSave() {}, prShowSave() {}, note() {}, othersRequestAi() {}, cardDialog: {},
    // What the page is on, as pr-panel.js says it: the selected review request, asked through the list's endpoint.
    prShown: () => ({ id: 'acme/widget#1', rec, kind: 'pr', ask: mode => c.othersRequestAi('acme/widget#1', mode) }),
    secTitle: t => t, panelBtnsHtml: () => '',
  });
  const files = ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'];
  vm.runInContext(files.map(f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8')).join('\n'), c);
  vm.runInContext([
    'const prView = { drafts: new Map() };', fn('prDraftsEntry'), fn('prEditDrafts'), cut(/^const prDoc = [^\n]*;/m), cut(/^const prSaveState = [^\n]*;/m), fn('prDraftsBodyHtml'), fn('prAct'),
  ].join('\n'), c);
  const e = vm.runInContext(`prDraftsEntry('acme/widget#1')`, c);
  Object.assign(e, { doc, saved: true, loaded: true });
  return { w, c, e };
}
test('a proposal made after the file exists is counted in the notice, and 「新しい下書きを足す」 adds it after what is written', () => {
  const proposal = { sha: 'b'.repeat(40), mode: 'split', drafts: [], overall: { body: 'split plan' } };
  const doc = { version: 1, fromSha: 'a'.repeat(40), drafts: [{ key: 'p1', source: 'person', path: 'a.rs', body: 'mine' }], overall: { body: 'my note' } };
  const { w, c, e } = offerWorld(doc, proposal);
  const html = vm.runInContext(`prDraftsBodyHtml({ id: 'acme/widget#1', url: 'u' }, ${JSON.stringify(proposal)}, 'panel')`, c);
  assert.ok(html.includes('下読みし直した下書きが 1 件あります'), html.slice(0, 400));
  assert.strictEqual(vm.runInContext(`prAct(${'{ target: { closest: s => s.includes("data-pr-take-new") ? { matches: () => false } : null } }'})`, c), true);
  assert.strictEqual(e.doc.overall.body, 'my note\n\nsplit plan');
  assert.strictEqual(e.doc.fromSha, proposal.sha);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.redrawn[0])), { sel: '[data-pr-overall]', inDialog: false });
  // An empty overall comment takes it as it is; one that holds it already is not offered.
  const empty = offerWorld({ ...doc, overall: { body: '' } }, proposal);
  vm.runInContext(`prAct({ target: { closest: s => s.includes("data-pr-take-new") ? { matches: () => false } : null } })`, empty.c);
  assert.strictEqual(empty.e.doc.overall.body, 'split plan');
  const held = offerWorld({ ...doc, overall: { body: 'split plan' } }, proposal);
  assert.ok(!vm.runInContext(`prDraftsBodyHtml({ id: 'acme/widget#1', url: 'u' }, ${JSON.stringify(proposal)}, 'panel')`, held.c).includes('下読みし直した下書き'));
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

test('a full read-through at the head a proposal was made at offers its drafts, and the file then says it is of that read', () => {
  const full = { sha: 'a'.repeat(40), drafts: [{ path: 'a.rs', line: 1, side: 'right', body: 'from the read' }], overall: { body: 'summary' } };
  const doc = { version: 1, fromSha: 'a'.repeat(40), fromMode: 'split', drafts: [], overall: { body: 'split plan' } };
  const { c, e } = offerWorld(doc, full);
  const html = vm.runInContext(`prDraftsBodyHtml({ id: 'acme/widget#1', url: 'u' }, ${JSON.stringify(full)}, 'panel')`, c);
  // The drafts only: the summary of a plain read is not offered.
  assert.ok(html.includes('下読みし直した下書きが 1 件あります'), html.slice(0, 300));
  vm.runInContext('prAct({ target: { closest: s => s.includes("data-pr-take-new") ? { matches: () => false } : null } })', c);
  assert.strictEqual(e.doc.drafts.length, 1);
  assert.strictEqual(e.doc.overall.body, 'split plan');
  assert.ok(!('fromMode' in e.doc));
  // The same read again is not news.
  const same = offerWorld({ ...doc, fromMode: undefined, drafts: [{ key: 'r1', source: 'reader', ...full.drafts[0] }] }, full);
  assert.ok(!vm.runInContext(`prDraftsBodyHtml({ id: 'acme/widget#1', url: 'u' }, ${JSON.stringify(full)}, 'panel')`, same.c).includes('下読みし直した'));
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
