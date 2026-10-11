/* The page's keeping of a PR's drafts (src/ui/pr-panel.js) against a fake `boardApi`, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'pr-panel.js'), 'utf8');
const cut = re => { const m = src.match(re); assert.ok(m, `not found: ${re}`); return m[0]; };
const fn = name => cut(new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

function world({ stored = null } = {}) {
  const w = { notes: [], redrawn: 0, posts: [], gets: 0, stored, gate: null, selected: 'pr:acme/widget#1' };
  const c = vm.createContext({
    structuredClone, CSS: { escape: x => x }, PR_BOX_SEL: '.pr-draft-body, .pr-overall', JSON, Promise, clearTimeout, setTimeout: () => 1, console,
    get selectedTaskId() { return w.selected; }, PR_REF: 'pr:', PR_SAVE_MS: 800,
    note: (...a) => { w.notes.push(a); }, prRedrawNow: () => { w.redrawn++; },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => ({ id: 'acme/widget#1' }), prReadOf: () => c.read,
    prNextKey: (doc, prefix) => { let n = 1; while (doc.drafts.some(d => d.key === `${prefix}${n}`)) n++; return `${prefix}${n}`; },
    tp: () => null, cardDialogEl: () => null, renderTaskPanel() {},
    prSeedDrafts: read => ({ version: 1, fromSha: read?.sha ?? null, drafts: (read?.drafts || []).map((d, i) => ({ key: `r${i + 1}`, source: 'reader', ...d })), overall: { body: '' } }),
    boardApi: async (base, url, o) => {
      if (!o) { w.gets++; if (!w.stored) throw Object.assign(new Error('no drafts'), { status: 404 }); return w.stored; }
      w.posts.push(JSON.parse(o.body));
      if (w.gate) await w.gate.promise;
      return {};
    },
  });
  vm.runInContext(['prView', 'prDraftsEntry', 'prDocOfStored', 'prEnsureDrafts', 'prEditDrafts', 'prScheduleSave', 'prSave', 'prShowSave', 'prSaveState', 'prInput'].map(n => {
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
  run(`prEnsureDrafts('acme/widget#1')`); await tick();
  c.read = read;
  run(`prEditDrafts('acme/widget#1', read, d => { d.drafts[0].body = 'unsaved'; })`);
  // What renderPrPanel does for a new selection: reload only when clean.
  run(`{ const e = prDraftsEntry('acme/widget#1'); if (!e.dirty && !e.saving) { e.loaded = false; e.error = null; } prEnsureDrafts('acme/widget#1'); }`);
  await tick();
  assert.strictEqual(w.gets, 1);
  assert.strictEqual(run(`prDoc('acme/widget#1', read).drafts[0].body`), 'unsaved');
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
