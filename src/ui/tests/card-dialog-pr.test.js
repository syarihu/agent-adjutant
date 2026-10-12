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
    prReadOf: () => w.read, prReadEntry: () => w.readEntry, prDraftsBodyHtml: () => w.dock, PR_HOOKS: { diff: null },
    cardDialogEl: () => dlg, cardDialogContent: () => content, cardDialogDockEl: () => col,
    cardDialogStrip() {}, cardDialogFill: () => ({}), cardDialogFocusOf: () => null, cardDialogFocusBack() {}, cardDialogCardEl: () => null,
    cardDialogMarkUpdated: () => { w.marks++; }, cardDialogShowUpdate: ch => { w.shown.push(ch); }, cardDialogPruneUpdate: () => { w.pruned++; }, cardDialogSpy() {},
    clearTimeout() {}, secTitle: t => `<div class="tp-sec-title">${t}</div>`,
  });
  vm.runInContext([
    read('util.js'), read('my-work-seen.js'), read('my-work-others.js'),
    read('pr-read.js'), read('pr-diff.js'),
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^const cardDialogSigOf = [^\n]*;/m), cut(dlgSrc, /^const cardDialogCardSigs = [^\n]*;/m),
    cut(read('task-view.js'), /^const HISTORY_LOADING = [^\n]*;/m), cut(read('task-view.js'), /^const DIFF_LOADING = [^\n]*;/m),
    ...['cardDialogChanged', 'cardDialogTaskHtml'].map(n => fnSrc(dlgSrc, n)),
    cut(read('pr-panel.js'), /^const prBoxFocused = [^\n]*;/m),
    fnSrc(read('pr-dialog.js'), 'prFocusSel'), fnSrc(read('pr-dialog.js'), 'prDialogSync'), fnSrc(read('pr-dialog.js'), 'prDialogPatch'), fnSrc(read('pr-dialog.js'), 'keepReadingPlace'), fnSrc(read('pr-dialog.js'), 'prDialogWithoutDiff'),
    cut(read('pr-dialog.js'), /^const prDialogDrawn = [^\n]*;/m),
    cut(read('pr-dialog.js'), /^const PR_DIALOG_GONE = [^\n]*;\nconst PR_DIALOG_NO_READ = [^\n]*;\nconst PR_DIALOG_NO_SPLIT = [^\n]*;/m),
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

/* The annotated diff's cards (pr-diff-view.js sets the hook; a stand-in is used here). */
const DIFF_LOADING_TEXT = '<div class="panel"><div class="empty-state">差分を読み込み中…</div></div>';
function withDiff(w, c, state) {
  state.loading = false;
  state.paths = [];
  state.drawn = 0;
  state.target = 0;
  state.order = [];
  c.PR_HOOKS.diff = {
    dialogPaths: () => state.paths,
    dialogHeadHtml: () => (state.loading ? DIFF_LOADING_TEXT : '<div>bar</div>'),
    dialogFileHtml: (r, rd, p) => `<section>${p}</section>`,
    dialogLabel: (r, p) => ({ label: p.split('/').pop(), title: p }),
    onDrawn: () => { state.drawn++; state.order.push('drawn'); },
    applyTarget: () => { state.target++; state.order.push('target'); },
  };
}

test('the diff arriving in an open dialog is its first fill, not a change; its cards come from the hook, with titles on the index', () => {
  const { w, c, sync, state } = world();
  const d = {};
  withDiff(w, c, d);
  d.loading = true;
  sync(true, true);
  assert.strictEqual(state('JSON.stringify(cardDialog.groups.map(g => g.key))'), '["top","diff"]');
  d.loading = false;
  d.paths = ['src/a/mod.rs', 'docs/x.md'];
  sync();
  assert.deepStrictEqual(w.shown, []);
  assert.strictEqual(state('cardDialog.updated.size'), 0);
  assert.strictEqual(state('JSON.stringify(cardDialog.groups.find(g => g.key === "diff").cards.map(c => [c.key, c.label, c.title]))'),
    JSON.stringify([['diff-bar', '差分の概要', ''], ['diff:src/a/mod.rs', 'mod.rs', 'src/a/mod.rs'], ['diff:docs/x.md', 'x.md', 'docs/x.md']]));
  // The index button of a file carries the whole path as its title.
  const html = state('cardDialogTaskHtml(cardDialog.groups).index');
  assert.ok(html.includes('data-cd-go-card="diff:src/a/mod.rs" title="src/a/mod.rs"'), html);
});

test('onDrawn runs right after the content is replaced, and the target is applied after the position is back', () => {
  const { w, c, sync } = world();
  const d = {};
  withDiff(w, c, d);
  sync(true, true);
  d.order.length = 0;
  w.read = readThrough('Another purpose');
  sync();
  assert.deepStrictEqual(d.order, ['drawn', 'target']);
  // Nothing changed: nothing is drawn, and nothing is looked at.
  d.order.length = 0;
  sync();
  assert.deepStrictEqual(d.order, []);
});

test('a change of the drafts column alone does not replace the content, however many files the diff has', () => {
  const { w, c, col, sync } = world();
  const d = {};
  withDiff(w, c, d);
  d.paths = ['a.rs', 'b.rs'];
  sync(true, true);
  const draws = w.contentDraws;
  const drawn = d.drawn;
  w.dock = 'DOCK v2';
  sync();
  assert.strictEqual(col.innerHTML, 'DOCK v2');
  assert.strictEqual(w.contentDraws, draws);
  assert.strictEqual(d.drawn, drawn);
});

/* A stand-in for the dialog's content: groups as sections, each with a title and cards, that can be replaced one by one. */
function fakeContent(groups, make = (g, c) => ({ key: c.key, html: c.html })) {
  const sections = groups.map(g => {
    const cards = g.cards.map(c => make(g, c));
    const section = { key: g.key, title: { html: g.label, replaceWith(n) { section.title = n; } }, cards,
      querySelector: () => section.title, querySelectorAll: () => [...section.cards] };
    cards.forEach(card => { card.replaceWith = n => { section.cards[section.cards.indexOf(card)] = n; }; });
    return section;
  });
  return sections;
}
const cardsOf = html => [{ key: 'top:facts', html: html.facts }, { key: 'diff:a.rs', html: '<section>a</section>' }];
const groupsOf = facts => [{ key: 'top', label: '要点', cards: [cardsOf({ facts })[0]] }, { key: 'diff', label: '差分', cards: [cardsOf({ facts })[1]] }];

test('only the cards whose markup changed are replaced: a filled diff file keeps its node when a time label changes', () => {
  const { c, state } = world();
  let whole = 0;
  const content = { children: [], replaceChildren(f) { whole++; this.children = f; } };
  const draw = facts => {
    const groups = groupsOf(facts);
    const frag = fakeContent(groups);
    frag.children = frag;
    c.prDialogPatch(content, frag, groups);
  };
  draw('5分前');
  assert.strictEqual(whole, 1);
  content.children = fakeContent(groupsOf('5分前'));
  const diffNode = content.children[1].cards[0];
  diffNode.filled = true;
  const factsNode = content.children[0].cards[0];
  draw('6分前');
  assert.strictEqual(whole, 1, 'not replaced as a whole');
  assert.notStrictEqual(content.children[0].cards[0], factsNode, 'the card with the time is new');
  assert.strictEqual(content.children[0].cards[0].html, '6分前');
  assert.strictEqual(content.children[1].cards[0], diffNode, 'the diff file is the same node');
  assert.strictEqual(diffNode.filled, true);
  // Another set of cards is a whole new content.
  const more = [...groupsOf('6分前'), { key: 'more', label: 'x', cards: [{ key: 'more:a', html: 'a' }] }];
  const frag = fakeContent(more);
  frag.children = frag;
  c.prDialogPatch(content, frag, more);
  assert.strictEqual(whole, 2);
  // A new dialog starts from nothing.
  state("prDialogDrawn.order = ''");
  draw('6分前');
  assert.strictEqual(whole, 3);
});

test('prDialogPatch keeps what it was last drawn from in the object it is given, so a second content does not share the dialog\'s', () => {
  const { c, state } = world();
  let whole = 0;
  const content = { children: [], replaceChildren(f) { whole++; this.children = f; } };
  const own = { order: '', html: new Map() };
  c.own = own;
  const groups = groupsOf('5分前');
  const frag = fakeContent(groups);
  frag.children = frag;
  c.prDialogPatch(content, frag, groups, own);
  assert.strictEqual(whole, 1);
  assert.deepStrictEqual([...own.html.keys()], ['top:facts', 'diff:a.rs']);
  assert.strictEqual(state('prDialogDrawn.order'), '', 'the dialog\'s own record is not touched');
  content.children = fakeContent(groups);
  const again = fakeContent(groupsOf('6分前'));
  again.children = again;
  c.prDialogPatch(content, again, groupsOf('6分前'), own);
  assert.strictEqual(whole, 1, 'patched against its own record, not replaced');
  assert.strictEqual(content.children[0].cards[0].html, '6分前');
});

test('keepReadingPlace puts back the open details and the card at the top, and only the details when asked to', () => {
  const { c } = world();
  const detail = open => ({ open });
  const card = (key, top, details) => ({ dataset: { cdCard: key }, querySelectorAll: () => details, getBoundingClientRect: () => ({ top, bottom: top + 100 }) });
  const a = card('a', -150, [detail(true)]);
  const b = card('b', 20, [detail(false), detail(true)]);
  const body = { scrollTop: 300, getBoundingClientRect: () => ({ top: 0 }) };
  const content = { querySelectorAll: () => [a, b] };
  c.args = [body, content];
  const restore = vm.runInContext('keepReadingPlace(...args)', c);
  // The page is drawn anew: details closed, the scroll lost, the card moved by 60px.
  a.querySelectorAll = () => [detail(false)];
  const fresh = [detail(false), detail(false)];
  b.querySelectorAll = () => fresh;
  body.scrollTop = 0;
  b.getBoundingClientRect = () => ({ top: 80, bottom: 180 });
  restore(false);
  assert.strictEqual(fresh[1].open, true);
  assert.strictEqual(body.scrollTop, 0, 'asked for the details only');
  restore();
  assert.strictEqual(body.scrollTop, 360, 'the scroll, then the card that was at the top back where it was (20px under the top)');
  // Where nothing can be measured (a dialog still closed) only the scroll top is kept.
  body.scrollTop = 40;
  const dead = vm.runInContext('keepReadingPlace', c)(body, content, false);
  body.scrollTop = 0;
  fresh[1].open = false;
  dead();
  assert.strictEqual(body.scrollTop, 40);
  assert.strictEqual(fresh[1].open, false);
});

test('a re-read arriving while the diff loads marks the changed cards of the read, and the diff arriving after is not marked', () => {
  const { w, c, sync, state } = world();
  const d = {};
  withDiff(w, c, d);
  sync(true, true);
  // The read-through is replaced (a re-read) and its diff is on its way again.
  d.loading = true;
  w.read = readThrough('Another purpose');
  sync();
  assert.strictEqual(w.shown.length, 1);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.shown[0].cards)), ['top:three']);
  // The diff comes: its cards are the diff's own, not news.
  d.loading = false;
  d.paths = ['a.rs'];
  sync();
  assert.strictEqual(w.shown.length, 1);
  assert.strictEqual(state('JSON.stringify([...cardDialog.updated])'), '["top:three"]');
});

test('a scroll by hand also ends the wait of the diff\'s own link', () => {
  const calls = [];
  const c = vm.createContext({ cardDialog: { target: 'x', pinned: 'y' }, PR_HOOKS: { diff: { onMoved: () => calls.push('moved') } } });
  vm.runInContext(cut(dlgSrc, /^const cardDialogUserMoved = [^\n]*/m) + '\ncardDialogUserMoved();', c);
  assert.deepStrictEqual(calls, ['moved']);
  assert.strictEqual(c.cardDialog.target, null);
});

test('a stored read that cannot be fetched does not hide the buttons of the out-of-review card in the dialog', () => {
  const { w, sync, state } = world();
  w.rec = rec({ files: [{ path: 'src/big.rs', additions: 2000, deletions: 0, change: 'modified' }], ai: { status: 'ready', sha: A, stale: true } });
  w.read = undefined;
  sync(true, true);
  const html = () => state('cardDialog.groups.flatMap(g => g.cards).map(c => c.html).join("")');
  assert.ok(!html().includes('data-pr-split'));
  w.readEntry = { error: 'boom' };
  sync(true);
  assert.ok(html().includes('data-pr-split') && html().includes('data-pr-ai'));
});
