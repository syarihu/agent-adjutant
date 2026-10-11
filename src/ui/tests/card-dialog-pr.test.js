/* The 「拡大」 dialog of a PR waiting on your review (src/ui/pr-dialog.js) against a small stand-in for the dialog it fills: the
   first fill marks nothing, a change while it is open is marked, and the column of drafts is never drawn under the caret.
   Run with `node --test`. The task dialog's own machinery (card-dialog.js) is cut out of its file and run as it is. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const dlgSrc = read('card-dialog.js');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const fnSrc = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

const A = 'a'.repeat(40);
const rec = (extra = {}) => ({ id: 'acme/widget#12', repo: 'acme/widget', number: 12, title: 'Round', state: 'ai-ready', files: [], additions: 1, deletions: 0, changedFiles: 1,
  ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z' }, ...extra });
const readThrough = (purpose = 'P') => ({ sha: A, three: { purpose, approach: 'Ap', impact: 'Im', essence: 'Es' }, points: [], order: [], deleted: [], decisions: [], tests: {}, unwritten: [] });

function world() {
  const w = { rec: rec(), read: readThrough(), dock: 'DOCK v1', shown: [], marks: 0, pruned: 0, contentDraws: 0, closed: 0, focus: null, others: { doc: { hub: null }, asking: new Set() } };
  const titleEl = { textContent: 'Round' };
  const body = { scrollTop: 0, getBoundingClientRect: () => ({ top: 0 }), focus() {} };
  const index = { innerHTML: '' };
  const content = { querySelectorAll: () => [], replaceChildren() { w.contentDraws++; } };
  const col = { innerHTML: '', hidden: true, scrollTop: 0, contains: el => !!el?.inDock, querySelector: () => null };
  const dlg = { open: true, close() { this.open = false; w.closed++; },
    querySelector: sel => sel === '#card-dialog-title' ? titleEl : sel === '.card-dialog-body' ? body : sel === '.card-dialog-index' ? index : null };
  const c = vm.createContext({
    selectedTaskId: 'pr:acme/widget#12', document: { get activeElement() { return w.focus; } }, tp: () => ({ hidden: false }), esc: x => String(x), CSS: { escape: x => x },
    prView: { force: false }, prFocusKeep: () => null, prRestoreFocus: () => null, others: w.others, othersById: () => w.rec, othersRecords: () => [w.rec], othersHubView: () => ({ canRead: true, why: '' }), othersNowEst: () => 0,
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3),
    prReadOf: () => w.read, prDraftsBodyHtml: () => w.dock, PR_HOOKS: { diff: null },
    cardDialogEl: () => dlg, cardDialogContent: () => content, cardDialogDockEl: () => col,
    cardDialogStrip() {}, cardDialogFill: () => ({}), cardDialogFocusOf: () => null, cardDialogFocusBack() {}, cardDialogCardEl: () => null,
    cardDialogMarkUpdated: () => { w.marks++; }, cardDialogShowUpdate: ch => { w.shown.push(ch); }, cardDialogPruneUpdate: () => { w.pruned++; }, cardDialogSpy() {},
    clearTimeout() {}, secTitle: t => `<div class="tp-sec-title">${t}</div>`,
  });
  vm.runInContext([
    read('util.js'), read('my-work-seen.js'), read('my-work-others.js'),
    read('pr-read.js'),
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^const cardDialogSigOf = [^\n]*;/m), cut(dlgSrc, /^const cardDialogCardSigs = [^\n]*;/m),
    cut(read('task-view.js'), /^const HISTORY_LOADING = [^\n]*;/m), cut(read('task-view.js'), /^const DIFF_LOADING = [^\n]*;/m),
    ...['cardDialogChanged', 'cardDialogTaskHtml'].map(n => fnSrc(dlgSrc, n)),
    cut(read('pr-panel.js'), /^const prBoxFocused = [^\n]*;/m),
    fnSrc(read('pr-dialog.js'), 'prFocusSel'), fnSrc(read('pr-dialog.js'), 'prDialogSync'),
    cut(read('pr-dialog.js'), /^const PR_DIALOG_GONE = [^\n]*;\nconst PR_DIALOG_NO_READ = [^\n]*;/m),
    "cardDialog.mode = 'review'; cardDialog.subject = 'pr:acme/widget#12';",
  ].join('\n'), c);
  const sync = (...a) => { c.args = a; return vm.runInContext('prDialogSync(...args)', c); };
  const state = expr => vm.runInContext(expr, c);
  return { w, c, col, content, sync, state };
}

test('the first fill marks nothing, and what is drawn is the groups of the read-through', () => {
  const { w, col, content, sync, state } = world();
  sync(true, true);
  assert.deepStrictEqual(w.shown, []);
  assert.strictEqual(state('cardDialog.updated.size'), 0);
  assert.strictEqual(w.contentDraws, 1);
  assert.strictEqual(col.innerHTML, 'DOCK v1');
  assert.strictEqual(col.hidden, false);
  assert.deepStrictEqual(state('JSON.stringify(cardDialog.groups.map(g => g.key))'), '["top"]');
  // Nothing changed: nothing is drawn again, and nothing is marked.
  sync();
  assert.strictEqual(w.contentDraws, 1);
  assert.deepStrictEqual(w.shown, []);
});

test('a change of what the reader said marks that card and tells the person', () => {
  const { w, sync, state } = world();
  sync(true, true);
  w.read = readThrough('Another purpose');
  sync();
  assert.strictEqual(w.shown.length, 1);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.shown[0].cards)), ['top:three']);
  assert.strictEqual(state('JSON.stringify([...cardDialog.updated])'), '["top:three"]');
  assert.ok(w.marks >= 1);
});

test('a box of the column with the caret in it holds the column\'s redraw, and the text in it is not touched', () => {
  const { w, col, sync, state } = world();
  sync(true, true);
  const box = { inDock: true, matches: sel => sel.includes('.pr-draft-body'), dataset: {} };
  w.focus = box;
  w.dock = 'DOCK v2 (typed text is in the document now)';
  w.read = readThrough('Changed while typing');
  sync();
  // The column stays as it was and is owed a redraw; the content, which has no box, goes on following.
  assert.strictEqual(col.innerHTML, 'DOCK v1');
  assert.strictEqual(state('cardDialog.held'), true);
  assert.strictEqual(w.shown.length, 1);
  // Focus leaves: the redraw is made.
  w.focus = null;
  sync();
  assert.strictEqual(col.innerHTML, 'DOCK v2 (typed text is in the document now)');
  assert.strictEqual(state('cardDialog.held'), false);
});

test('a PR that is gone says so and keeps what it showed; a panel that moved on closes the dialog quietly', () => {
  const { w, c, sync, state } = world();
  sync(true, true);
  w.rec = null;
  let stripped = '';
  c.cardDialogStrip = text => { stripped = text; };
  sync();
  assert.ok(stripped.includes('この PR がなくなりました'));
  assert.strictEqual(w.closed, 0);
  w.rec = rec();
  c.selectedTaskId = 'pr:other/repo#1';
  sync();
  assert.strictEqual(w.closed, 1);
  assert.strictEqual(state('cardDialog.quiet'), true);
});

test('a read-through that arrives after the dialog was opened is the first fill of its cards, not a change of them', () => {
  const { w, sync, state } = world();
  w.read = undefined;
  sync(true, true);
  w.read = readThrough();
  sync();
  assert.deepStrictEqual(w.shown, []);
  assert.strictEqual(state('cardDialog.updated.size'), 0);
  // Once it is there, a change of it is marked as usual.
  w.read = readThrough('Another purpose');
  sync();
  assert.strictEqual(w.shown.length, 1);
});
