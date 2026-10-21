/* The page's half of the annotated diff (src/ui/pr-diff-view.js) against stand-ins for the page, run with `node --test`: what is
   fetched and for whom, what is drawn and when, where a link goes, and what 「＋」 and the annotation's buttons do. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const SHA = 'a'.repeat(40);
const DIFF = [
  'diff --git a/src/a.rs b/src/a.rs', '--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,2 +1,3 @@', ' fn a() {', '-    old();', '+    // why', '+    new();', ' }',
  'diff --git a/docs/x.md b/docs/x.md', 'new file mode 100644', '--- /dev/null', '+++ b/docs/x.md', '@@ -0,0 +1 @@', '+hello', '',
].join('\n');
const rec = (id = 'acme/widget#1') => ({ id, repo: 'acme/widget', number: 1, url: 'https://github.com/acme/widget/pull/1', headSha: SHA, files: [] });
const readThrough = (extra = {}) => ({
  sha: SHA, madeAt: 'm1', order: [{ path: 'src/a.rs', category: 'logic' }], deleted: [],
  annotations: [{ path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'Explains new' }, { path: 'src/a.rs', anchor: '// why', kind: 'tr', text: 'Translated' }], ...extra,
});

/* A stand-in element: what the code under test reads and writes. */
function el(dataset = {}, extra = {}) {
  const classes = new Set();
  return Object.assign({
    dataset, hidden: false, innerHTML: '', attrs: {}, textContent: '', isConnected: true, offsetWidth: 0,
    classList: { toggle: (c, on) => (on ? classes.add(c) : classes.delete(c)), add: c => classes.add(c), remove: c => classes.delete(c), contains: c => classes.has(c) },
    setAttribute(k, v) { this.attrs[k] = v; }, toggleAttribute(k, on) { this.attrs[k] = on; },
    closest: () => null, querySelector: () => null, querySelectorAll: () => [], matches: () => false, scrollIntoView() {},
  }, extra);
}
const wrap = el => el;

function world({ io = false } = {}) {
  const w = { calls: [], gets: [], pending: [], notes: [], edits: [], drafts: [], hubView: { canRead: true }, selected: 'pr:acme/widget#1', pane: 'detail', dialogOpen: false, loaded: true };
  // The middle's root and the dialog's content, each with the pieces the code looks for.
  const mk = () => {
    const bodies = [el({ prdBody: '0' }), el({ prdBody: '1' })];
    const sections = bodies.map((b, i) => el({ prdFile: String(i) }, { querySelector: s => (s === '[data-prd-body]' ? b : null), closest: () => null }));
    const root = el({}, { querySelectorAll: s => (s === '[data-prd-body]' ? bodies : s === '[data-prd-file]' ? sections : []),
      querySelector: s => (s === '[data-prd-file]' ? sections[0] : s.startsWith('[data-prd-file=') ? sections[Number(s.match(/"(\d+)"/)[1])] : null) });
    return { root, bodies, sections };
  };
  w.panel = mk();
  w.dialog = mk();
  // The middle: what is drawn in it is `w.panel`, and its body is what scrolls.
  const scroll = el({}, { scrollTop: 0 });
  w.byId = { 'task-read': el({}, { hidden: false }), 'task-read-content': w.panel.root, 'task-read-body': scroll };
  w.scrollEl = scroll;
  const body = el({});
  const dlg = { get open() { return w.dialogOpen; }, querySelector: s => (s === '.card-dialog-body' ? body : null), addEventListener: (t, f, cap) => { w.dlgClick = f; } };
  const c = vm.createContext({
    TextEncoder, TextDecoder, CSS: { escape: x => x }, console, Promise, Date, setTimeout: f => 0, structuredClone, document: { getElementById: id => w.byId?.[id] || null },
    IntersectionObserver: io ? class { constructor(cb, o) { w.io = { cb, o, seen: [] }; } observe(x) { w.io.seen.push(x); } unobserve() {} disconnect() { w.ioGone = true; } } : undefined,
    get selectedTaskId() { return w.selected; }, PR_REF: 'pr:', PR_HOOKS: { diff: null }, DIFF_LOADING: '<div class="loading">差分を読み込み中…</div>',
    secTitle: t => t, panelBtnsHtml: j => j,
    cardDialogEl: () => dlg, cardDialogContent: () => Object.assign(w.dialog.root, { addEventListener() {} }),
    cardDialog: { mode: 'review', subject: 'pr:acme/widget#1', quiet: false }, CARD_DIALOG_TARGET_MS: 10000,
    cardDialogCardEl: key => { w.calls.push(['cardEl', key]); return key === 'diff:src/a.rs' ? w.dialog.sections[0] : key === 'diff:docs/x.md' ? w.dialog.sections[1] : null; },
    cardDialogGo: (...a) => { w.calls.push(['go', ...a, { filled: w.dialog.bodies[1].innerHTML !== '' }]); },
    closeCardDialog: () => { w.calls.push(['close']); w.dialogOpen = false; }, openPrDialog: opener => { w.calls.push(['open', opener]); w.dialogOpen = true; },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3), othersById: () => rec(), prReadOf: () => w.read,
    othersHubView: () => w.hubView, othersRecords: () => [], others: { doc: { hub: null }, asking: new Set() },
    renderTaskPanel: () => { w.calls.push(['render']); },
    // What the page is on, as pr-panel.js says it: the selected review request.
    prShown: () => w.shown !== undefined ? w.shown : (typeof w.selected === 'string' && w.selected.startsWith('pr:') ? { id: w.selected.slice(3), rec: rec(w.selected.slice(3)), kind: 'pr', hubView: w.hubView, asking: false } : null),
    prRecOf: () => rec(), prRedrawFor: id => { if (w.selected === `pr:${id}`) w.calls.push(['render']); },
    cardDialogOnPr: () => ['review', 'own-read'].includes(c.cardDialog.mode),
    prDraftsEntry: () => ({ loaded: w.loaded }), prDoc: () => ({ drafts: w.drafts }),
    prEditDrafts: (id, rd, fn) => { if (!w.loaded) return false; const doc = { drafts: [...w.drafts] }; fn(doc); w.edits.push(doc.drafts[doc.drafts.length - 1]); return true; },
    prRedrawNow: f => { w.calls.push(['redraw', f]); }, go: (p, o) => { w.calls.push(['gopane', p, o]); }, nav: { get pane() { return w.pane; } },
    note: (...a) => { w.notes.push(a); }, prAsk: p => { w.calls.push(['ask', p]); },
    boardApi: (base, url) => new Promise((resolve, reject) => { w.gets.push(url); w.pending.push({ resolve, reject }); }),
  });
  vm.runInContext(['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'].map(read).join('\n'), c);
  vm.runInContext(read('pr-diff-view.js'), c);
  const run = e => vm.runInContext(e, c);
  w.read = readThrough();
  return { w, c, run };
}
const tick = () => new Promise(r => setImmediate(r));
const ok = (w, run, i = 0) => { w.pending[i].resolve({ id: 'acme/widget#1', sha: SHA, bytes: DIFF.length, diff: DIFF }); return tick(); };

test('a diff is applied only to its own PR and its own read-through', async () => {
  const { w, c, run } = world();
  const a = c.rec_ = rec();
  c.prDiffEnsure(rec(), w.read);
  c.prDiffEnsure(rec('acme/widget#2'), w.read);
  assert.deepStrictEqual(w.gets, ['/api/others/diff?id=acme%2Fwidget%231', '/api/others/diff?id=acme%2Fwidget%232']);
  // The first PR's read-through is replaced while its diff is on its way: the answer is for the old one and is not taken.
  const newer = readThrough({ madeAt: 'm2' });
  assert.strictEqual(c.prDiffEnsure(rec(), newer).state, 'loading');
  await ok(w, run, 0);
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').state`), 'loading');
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').key`), null);
  // The other PR's own answer is taken.
  await ok(w, run, 1);
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#2').state`), 'ok');
  // The next round asks for the newer one, and that answer is taken.
  c.prDiffEnsure(rec(), newer);
  assert.strictEqual(w.gets.length, 3);
  await ok(w, run, 2);
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').state`), 'ok');
  // Held: asked again for nothing.
  c.prDiffEnsure(rec(), newer);
  assert.strictEqual(w.gets.length, 3);
});

test('no diff stored, an empty one and a too-large one are answers, and a failure is asked again later', async () => {
  const states = async answer => {
    const { w, c, run } = world();
    c.prDiffEnsure(rec(), w.read);
    answer(w.pending[0]);
    await tick();
    return { w, c, run, state: run(`prDiff.cache.get('acme/widget#1').state`) };
  };
  assert.strictEqual((await states(p => p.reject(Object.assign(new Error('no diff'), { status: 404 })))).state, 'none');
  assert.strictEqual((await states(p => p.resolve({ sha: SHA, bytes: 0, diff: '' }))).state, 'empty');
  const big = await states(p => p.resolve({ sha: SHA, bytes: 9 * 1024 * 1024, diff: null }));
  assert.strictEqual(big.state, 'big');
  assert.ok(big.c.prdStatusHtml(rec(), big.run(`prDiff.cache.get('acme/widget#1')`)).includes('9.0 MB'));
  // A diff of another head is not this read-through's.
  assert.strictEqual((await states(p => p.resolve({ sha: 'b'.repeat(40), bytes: 3, diff: 'x\n' }))).state, 'none');
  assert.strictEqual((await states(p => p.resolve({ sha: SHA, bytes: 3, diff: 'not a diff\n' }))).state, 'bad');
  const failed = await states(p => p.reject(new Error('boom')));
  assert.strictEqual(failed.state, 'error');
  // Not at once again (a redraw would loop), and not forever.
  failed.c.prDiffEnsure(rec(), failed.w.read);
  assert.strictEqual(failed.w.gets.length, 1);
  failed.run(`prDiff.cache.get('acme/widget#1').retryAt = 0`);
  failed.c.prDiffEnsure(rec(), failed.w.read);
  assert.strictEqual(failed.w.gets.length, 2);
});

test('each state says what it is, and 404 offers 「下読みし直す」 and GitHub through the existing re-read button', async () => {
  const text = async answer => {
    const { w, c } = world();
    const html = () => c.prdDialogHeadHtml(rec(), w.read);
    assert.ok(html().includes('差分を読み込み中'));
    assert.ok(html().includes('class="loading"'), 'it says it with the loading note the dialog\'s change marks know');
    answer(w.pending[0]);
    await tick();
    return html();
  };
  const none = await text(p => p.reject(Object.assign(new Error('x'), { status: 404 })));
  assert.ok(none.includes('この下読みには差分が保存されていません。下読みし直すと表示されます') && none.includes('data-pr-reread') && none.includes('下読みし直す'));
  assert.ok(none.includes('href="https://github.com/acme/widget/pull/1/files"') && none.includes('GitHub で開く'));
  assert.ok((await text(p => p.resolve({ sha: SHA, bytes: 0, diff: '' }))).includes('差分はありません'));
  assert.ok((await text(p => p.resolve({ sha: SHA, bytes: 3 * 1024 * 1024, diff: null }))).includes('差分が大きすぎるため保存していません（3.0 MB）'));
  assert.ok((await text(p => p.reject(new Error('boom')))).includes('role="alert"'));
  const okHtml = await text(p => p.resolve({ sha: SHA, bytes: DIFF.length, diff: DIFF }));
  // The diff is there and the bar has nothing to say: no card at all.
  assert.strictEqual(okHtml, '');
  const stale = await (async () => {
    const { w, c } = world();
    w.pending.length = 0;
    const r = { ...rec(), headSha: 'c'.repeat(40) };
    c.prdDialogHeadHtml(r, w.read);
    w.pending[0].resolve({ sha: SHA, bytes: 1, diff: DIFF });
    await tick();
    return c.prdDialogHeadHtml(r, w.read);
  })();
  assert.ok(stale.includes('この差分は aaaaaaa で下読みしたときのものです') && stale.includes('data-pr-sec="diff-bar"'));
  // Files of the record that the diff lacks are said in the bar.
  const lacking = await (async () => {
    const { w, c } = world();
    w.pending.length = 0;
    const r = { ...rec(), files: [{ path: 'src/a.rs', additions: 1, deletions: 0 }, { path: 'lost.rs', additions: 1, deletions: 0 }] };
    c.prdDialogHeadHtml(r, w.read);
    w.pending[0].resolve({ sha: SHA, bytes: 1, diff: DIFF });
    await tick();
    return c.prdDialogHeadHtml(r, w.read);
  })();
  assert.ok(lacking.includes('data-pr-sec="diff-bar"') && lacking.includes('差分にないファイル 1 件') && !lacking.includes('data-prd-tab'));
  // The dialog's cards: the bar first, then the files in the diff's order, with short labels.
  const { w, c } = world();
  c.prdDialogHeadHtml(rec(), w.read);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(c.prdDialogPaths(rec(), w.read))), []);
  w.pending[0].resolve({ sha: SHA, bytes: 1, diff: DIFF });
  await tick();
  assert.deepStrictEqual(JSON.parse(JSON.stringify(c.prdDialogPaths(rec(), w.read))), ['src/a.rs', 'docs/x.md']);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(c.prdDialogLabel(rec(), 'src/a.rs'))), { label: 'a.rs', title: 'src/a.rs' });
  assert.strictEqual(c.prdDialogHeadHtml(rec(), w.read), '');
  assert.ok(c.prdDialogFileHtml(rec(), w.read, 'src/a.rs').includes('data-prd-file="0"'));
});

async function loaded(opts) {
  const x = world(opts);
  x.c.prDiffEnsure(rec(), x.w.read);
  await ok(x.w, x.run, 0);
  return x;
}

test('with no IntersectionObserver every file is drawn at once, from one render each', async () => {
  const { w, c, run } = await loaded();
  let renders = 0;
  const real = c.prDiffBodyHtml;
  c.prDiffBodyHtml = (...a) => { renders++; return real(...a); };
  c.prdSync('read');
  assert.ok(w.panel.bodies.every(b => b.innerHTML.includes('prd-l')));
  assert.strictEqual(renders, 2);
  // Drawn again: nothing is rendered again, and what is there is left alone.
  c.prdSync('read');
  assert.strictEqual(renders, 2);
});

test('a big diff is drawn file by file, as each comes near the screen, in the element that scrolls', async () => {
  const { w, c, run } = await loaded({ io: true });
  run(`prDiff.cache.get('acme/widget#1').layout.rows = ${3000}`);
  c.prdSync('read');
  assert.strictEqual(w.io.o.rootMargin, '1200px 0px');
  assert.strictEqual(w.io.seen.length, 2);
  assert.ok(w.panel.bodies.every(b => b.innerHTML === ''));
  const near = w.panel.bodies[1];
  w.io.cb([{ isIntersecting: true, target: near }, { isIntersecting: false, target: w.panel.bodies[0] }]);
  assert.ok(near.innerHTML.includes('prd-l'));
  assert.strictEqual(w.panel.bodies[0].innerHTML, '');
  assert.strictEqual(w.io.o.root, w.scrollEl, 'the middle\'s observer is rooted at the element that scrolls');
  // The dialog's observer is rooted at its own body, not at the middle's.
  w.dialogOpen = true;
  c.prdOnDrawn();
  assert.strictEqual(w.io.o.root, run(`cardDialogEl().querySelector('.card-dialog-body')`));
  assert.notStrictEqual(w.io.o.root, w.scrollEl);
});

test('after the dialog replaced its content, the files drawn before are drawn again from the cache', async () => {
  const { w, c, run } = await loaded({ io: true });
  run(`prDiff.cache.get('acme/widget#1').layout.rows = ${3000}`);
  w.dialogOpen = true;
  c.prdOnDrawn();
  w.io.cb([{ isIntersecting: true, target: w.dialog.bodies[0] }]);
  assert.ok(w.dialog.bodies[0].innerHTML.includes('prd-l'));
  let renders = 0;
  const real = c.prDiffBodyHtml;
  c.prDiffBodyHtml = (...a) => { renders++; return real(...a); };
  // The content is replaced: new, empty bodies.
  const fresh = [el({ prdBody: '0' }), el({ prdBody: '1' })];
  w.dialog.bodies.splice(0, 2, ...fresh);
  c.prdOnDrawn();
  assert.ok(fresh[0].innerHTML.includes('prd-l'), 'the one drawn before is drawn again');
  assert.strictEqual(fresh[1].innerHTML, '', 'the other waits to be seen');
  assert.strictEqual(renders, 0, 'from the cache, with no new render');
});

test('every file\'s card is shown, and there is no tab or fold to hide any', async () => {
  const { w, c, run } = await loaded();
  c.prdSync('read');
  assert.strictEqual(w.panel.sections[0].hidden, false);
  assert.strictEqual(w.panel.sections[1].hidden, false);
  assert.strictEqual(w.panel.root.classList.contains('prd-folded'), false);
  // The buttons of the old tabs and fold are nobody\'s any more.
  const e = sel => ({ target: { closest: s => (s === sel ? el({ prdTab: 'doc' }) : null) } });
  assert.strictEqual(c.prdAct(e('[data-prd-tab]'), {}), false);
  assert.strictEqual(c.prdAct(e('[data-prd-fold]'), {}), false);
  assert.strictEqual(run(`'cat' in prDiff || 'folded' in prDiff`), false);
});

test('a link draws the file, then goes there', async () => {
  const { w, c, run } = await loaded({ io: true });
  run(`prDiff.cache.get('acme/widget#1').layout.rows = ${3000}`);
  w.dialogOpen = true;
  c.prdGoto('docs/x.md', 1, { place: 'dialog' });
  assert.deepStrictEqual(w.calls.filter(x => x[0] === 'open'), [], 'the dialog is not opened by a link');
  const go = w.calls.find(x => x[0] === 'go');
  assert.strictEqual(go[1], 'diff:docs/x.md');
  assert.strictEqual(go[3], false);
  // At the moment of the jump the file's body was drawn.
  assert.strictEqual(go[4].filled, true);
  assert.strictEqual(run('prDiff.target'), null);
  // A file that is not in the diff goes to the group.
  w.calls.length = 0;
  c.prdGoto('nope.rs', null, { place: 'dialog' });
  assert.deepStrictEqual(w.calls.filter(x => x[0] === 'go').map(x => x.slice(1, 4)), [[null, 'diff', true]]);
});

test('a link waits for a diff that is on its way, and is dropped when there is none', async () => {
  const { w, c, run } = world();
  w.dialogOpen = true;
  c.prDiffEnsure(rec(), w.read);
  c.prdGoto('src/a.rs', 3, { place: 'dialog' });
  assert.strictEqual(w.calls.filter(x => x[0] === 'go').length, 0);
  assert.ok(run('prDiff.target'));
  await ok(w, run, 0);
  c.prdApplyTarget();
  assert.strictEqual(w.calls.filter(x => x[0] === 'go').length, 1);
  assert.strictEqual(run('prDiff.target'), null);
  // No diff stored: the group, and the wait is over.
  const n = world();
  n.w.dialogOpen = true;
  n.c.prDiffEnsure(rec(), n.w.read);
  n.c.prdGoto('src/a.rs', 3, { place: 'dialog' });
  n.w.pending[0].reject(Object.assign(new Error('x'), { status: 404 }));
  await tick();
  n.c.prdApplyTarget();
  assert.deepStrictEqual(n.w.calls.filter(x => x[0] === 'go').map(x => x.slice(1, 4)), [[null, 'diff', true]]);
});

const onLine = (c, attrs, extra = {}) => {
  const file = el({ prdFile: '0' });
  const btn = el(attrs, { closest: s => (s === '[data-prd-file]' ? file : s === '[data-prd-ann]' ? el({ prdAnn: '0' }) : null), matches: s => !!extra.matches?.includes(s) });
  return { target: { closest: s => s === extra.hit ? btn : null } };
};

test('「＋」 starts a draft of the person on the right side and line, and the middle goes to 下書き', async () => {
  const { w, c } = await loaded();
  c.prdAct(onLine(c, { prdPlus: 'left:2' }, { hit: '[data-prd-plus]' }), { place: 'read' });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.edits)), [{ key: 'p1', source: 'person', path: 'src/a.rs', line: 2, side: 'left', body: '' }]);
  const order = w.calls.map(x => x[0]);
  assert.ok(order.indexOf('gopane') < order.indexOf('redraw'), 'the tab first, then the box');
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'gopane')[1])), { pane: 'drafts' });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'redraw')[1])), { sel: '[data-pr-body="p1"]', inDialog: false });
});

test('in 「拡大」 the box appears in the column and the middle is left where it is', async () => {
  const { w, c } = await loaded();
  c.prdAct(onLine(c, { prdPlus: 'right:3' }, { hit: '[data-prd-plus]' }), { place: 'dialog' });
  assert.strictEqual(w.calls.some(x => x[0] === 'gopane'), false);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'redraw')[1])), { sel: '[data-pr-body="p1"]', inDialog: true });
  assert.strictEqual(w.edits[0].line, 3);
});

test('an explanation\'s 「下書きに」 is the same, with its text as the body; unanchored it is a file draft; unloaded drafts say so', async () => {
  const { w, c } = await loaded();
  const hit = '[data-prd-todraft], [data-prd-ask]';
  c.prdAct(onLine(c, { prdAt: 'right:3' }, { hit, matches: ['[data-prd-todraft]'] }), { place: 'read' });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.edits[0])), { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: 'Explains new' });
  c.prdAct(onLine(c, {}, { hit, matches: ['[data-prd-todraft]'] }), { place: 'read' });
  assert.strictEqual(w.edits[1].line, undefined);
  w.loaded = false;
  c.prdAct(onLine(c, { prdPlus: 'right:3' }, { hit: '[data-prd-plus]' }), { place: 'read' });
  assert.strictEqual(w.edits.length, 2);
  assert.strictEqual(w.notes.length, 1);
});

test('a concern\'s 「下書きに」 puts the reader\'s draft for the author in the box, not the note on the line', async () => {
  const { w, c, run } = world();
  w.read = readThrough({ annotations: [{ path: 'src/a.rs', anchor: 'new();', kind: 'concern', text: 'Why is this here?', draft: 'Could you say why this is here?' }] });
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  c.prdAct(onLine(c, { prdAt: 'right:3' }, { hit: '[data-prd-todraft], [data-prd-ask]', matches: ['[data-prd-todraft]'] }), { place: 'read' });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.edits[0])), { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: 'Could you say why this is here?' });
  // 「聞く」 asks about the note.
  c.prdAct(onLine(c, { prdAt: 'right:3' }, { hit: '[data-prd-todraft], [data-prd-ask]', matches: [] }), { place: 'read' });
  assert.strictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'ask')[1])).title, 'Why is this here?');
});

test('「聞く」 in 「拡大」 closes it first, then asks with the line and the first words', async () => {
  const { w, c, run } = await loaded();
  w.dialogOpen = true;
  c.prdAct(onLine(c, { prdAt: 'right:3' }, { hit: '[data-prd-todraft], [data-prd-ask]', matches: [] }), { place: 'dialog' });
  assert.deepStrictEqual(w.calls.filter(x => ['close', 'ask'].includes(x[0])).map(x => x[0]), ['close', 'ask']);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'ask')[1])), { path: 'src/a.rs', line: 3, title: 'Explains new' });
  assert.strictEqual(run('cardDialog.quiet'), true);
});

test('the hook keeps its shape', () => {
  const { run } = world();
  assert.deepStrictEqual(JSON.parse(run('JSON.stringify(Object.keys(PR_HOOKS.diff).sort())')), ['act', 'applyTarget', 'dialogFileHtml', 'dialogHeadHtml', 'dialogLabel', 'dialogPaths',
    'goto', 'onDrawn', 'onDrawnAt', 'onMoved']);
});

test('only the selected PR and a few others are kept, and what was drawn is dropped for the others', async () => {
  const { w, c, run } = await loaded();
  c.prdSync('read');
  assert.ok(run(`prDiff.cache.get('acme/widget#1').bodies.size`) > 0);
  for (const n of [2, 3, 4, 5, 6]) c.prDiffEnsure({ ...rec(`acme/widget#${n}`) }, w.read);
  // PR 1 is selected, so it stays with its drawn bodies; of the others the three used last stay.
  assert.deepStrictEqual(JSON.parse(run(`JSON.stringify([...prDiff.cache.keys()].sort())`)), ['acme/widget#1', 'acme/widget#4', 'acme/widget#5', 'acme/widget#6']);
  assert.ok(run(`prDiff.cache.get('acme/widget#1').bodies.size`) > 0);
  // Selecting another drops what PR 1 drew, and keeps its diff.
  w.selected = 'pr:acme/widget#6';
  c.prDiffEnsure(rec('acme/widget#6'), w.read);
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').bodies.size`), 0);
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').state`), 'ok');
});

test('a very long file opens by a click, and a link into it opens it', async () => {
  const { w, c, run } = await loaded();
  run(`(() => { const f = prDiff.cache.get('acme/widget#1').layout.files[1]; f.closed = true; })()`);
  c.prdSync('read');
  assert.strictEqual(w.panel.bodies[1].hidden, true);
  assert.strictEqual(w.panel.bodies[1].innerHTML, '');
  w.dialogOpen = true;
  c.prdGoto('docs/x.md', null, { place: 'dialog' });
  assert.strictEqual(w.dialog.bodies[1].hidden, false);
  assert.ok(w.dialog.bodies[1].innerHTML.includes('prd-l'));
});

test('a link waiting for its diff is dropped when the person scrolls by hand', async () => {
  const { w, c, run } = world();
  w.dialogOpen = true;
  c.prDiffEnsure(rec(), w.read);
  c.prdGoto('src/a.rs', 3, { place: 'dialog' });
  assert.ok(run('prDiff.target'));
  c.prdOnMoved();
  await ok(w, run, 0);
  c.prdApplyTarget();
  assert.strictEqual(w.calls.filter(x => x[0] === 'go').length, 0);
});

test('an error is retried by a redraw it schedules, for the PR still shown, and a new read-through starts without waiting', async () => {
  const { w, c, run } = world();
  const timers = [];
  c.setTimeout = (f, ms) => { timers.push([f, ms]); return 0; };
  c.prDiffEnsure(rec(), w.read);
  w.pending[0].reject(new Error('boom'));
  await tick();
  assert.strictEqual(run(`prDiff.cache.get('acme/widget#1').state`), 'error');
  assert.strictEqual(timers.length, 1);
  assert.strictEqual(timers[0][1], 5000);
  w.calls.length = 0;
  timers[0][0]();
  assert.deepStrictEqual(w.calls, [['render']]);
  // Another PR is shown by then: nothing is drawn.
  w.calls.length = 0;
  w.selected = 'pr:acme/widget#2';
  timers[0][0]();
  assert.deepStrictEqual(w.calls, []);
  // A new read-through is not held back by the failure of the old one.
  w.selected = 'pr:acme/widget#1';
  c.prDiffEnsure(rec(), readThrough({ madeAt: 'm2' }));
  assert.strictEqual(w.gets.length, 2);
});

/* The middle of a task's read-through is the third place the diff is drawn in: what is selected is a task, so `prShown` says `task`. */
function readWorld() {
  const x = world();
  const scroll = el({}, { scrollTop: 0 });
  x.w.byId = { 'task-read': el({}, { hidden: false }), 'task-read-content': x.w.dialog.root, 'task-read-body': scroll };
  x.w.shown = { id: 'acme/widget#1', rec: rec(), kind: 'task', hubView: x.w.hubView, asking: false };
  x.w.selected = 'task-1';
  x.scroll = scroll;
  x.run('PRD_VIEWS').read = {
    ready: () => true, place: 'read', go: (...a) => { x.w.calls.push(['rgo', ...a]); return true; },
    cardEl: key => (key === 'diff:docs/x.md' ? x.w.dialog.sections[1] : key === 'diff:src/a.rs' ? x.w.dialog.sections[0] : null),
  };
  return x;
}

test('the middle of a task is a place of its own: its root is the read-through\'s content, and what scrolls is its body', async () => {
  const { w, c, run, scroll } = readWorld();
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  const p = c.prdPlace('read');
  assert.strictEqual(p.root, w.dialog.root);
  assert.strictEqual(p.scroll, scroll);
  // Kept but not on screen (the other tab, or a review request selected): no place, so another PR's state does not reach it.
  w.byId['task-read'].hidden = true;
  assert.strictEqual(c.prdPlace('read'), null);
  w.byId['task-read'].hidden = false;
  // A review request's middle is the same place (its 差分 tab).
  w.shown = { ...w.shown, kind: 'pr' };
  assert.strictEqual(c.prdPlace('read').root, w.dialog.root);
  w.shown = { ...w.shown, kind: 'task' };
  // Nothing drawn in it yet: no place.
  w.byId['task-read-content'] = el({}, { querySelector: () => null });
  assert.strictEqual(c.prdPlace('read'), null);
  // The diff is the one of the task's PR, whatever the panel selects.
  w.byId['task-read-content'] = w.dialog.root;
  assert.strictEqual(c.prdCurrent().id, 'acme/widget#1');
  c.prdSync('read', true);
  assert.ok(w.dialog.bodies.every(b => b.innerHTML.includes('prd-l')), 'its files are drawn from the one cache');
  assert.ok(run(`prDiff.cache.get('acme/widget#1').filled.read.size`) === 2);
  // Another PR on show leaves it empty.
  w.shown = null;
  assert.strictEqual(c.prdCurrent(), null);
});

test('a link in the middle of a task goes there and opens no dialog, after the file is drawn', async () => {
  const { w, c, run } = readWorld();
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  c.prdGoto('docs/x.md', 1, { place: 'read', opener: el() });
  assert.deepStrictEqual(w.calls.filter(x => x[0] === 'open'), [], 'no 「拡大」 for it');
  const go = w.calls.find(x => x[0] === 'rgo');
  assert.deepStrictEqual(go.slice(1, 3), ['diff:docs/x.md', null]);
  assert.strictEqual(run('prDiff.target'), null);
  // Not in the diff: the group.
  w.calls.length = 0;
  c.prdGoto('nope.rs', null, { place: 'read' });
  assert.deepStrictEqual(w.calls.filter(x => x[0] === 'rgo').map(x => x.slice(1, 4)), [[null, 'diff', true]]);
  assert.ok(!w.calls.some(x => x[0] === 'go'), 'not the dialog\'s');
});

/* The same place for a review request: its 下読み tab is where the diff is, and 「拡大」 is the other place a link can land in. */
function prReadWorld() {
  const x = readWorld();
  x.w.shown = { id: 'acme/widget#1', rec: rec(), kind: 'pr', hubView: x.w.hubView, asking: false };
  x.w.selected = 'pr:acme/widget#1';
  x.w.pane = 'detail';
  return x;
}

test('a link from the 下書き tab of a review request goes back to the 下読み tab first; from 下読み it lands in the same view', async () => {
  const { w, c, run } = prReadWorld();
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  w.pane = 'drafts';
  c.prdGoto('docs/x.md', 1, { place: 'read', opener: el() });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'gopane').slice(1))), [{ pane: 'detail' }, { replace: true }]);
  assert.deepStrictEqual(w.calls.filter(x => x[0] === 'open'), [], 'no 「拡大」 for it');
  // Already on 下読み, which holds the diff: nothing to switch, and it lands.
  w.pane = 'detail';
  w.calls.length = 0;
  c.prdGoto('docs/x.md', 1, { place: 'read' });
  assert.ok(!w.calls.some(x => x[0] === 'gopane'));
  assert.deepStrictEqual(w.calls.find(x => x[0] === 'rgo').slice(1, 3), ['diff:docs/x.md', null]);
});

test('a link lands in 「拡大」 while it is open on the PR, else in the middle', async () => {
  const { w, c, run } = prReadWorld();
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  w.dialogOpen = true;
  c.prdGoto('docs/x.md', null, { place: 'dialog' });
  assert.ok(w.calls.some(x => x[0] === 'go'), 'the dialog');
  assert.ok(!w.calls.some(x => x[0] === 'rgo'));
  // Open on another PR: not that one's.
  w.calls.length = 0;
  c.cardDialog.subject = 'pr:acme/widget#2';
  c.prdGoto('docs/x.md', null, { place: 'read' });
  assert.ok(w.calls.some(x => x[0] === 'rgo') && !w.calls.some(x => x[0] === 'go'));
});

test('a link meant for the middle of a task is dropped when the task on show is another', async () => {
  const { w, c, run } = readWorld();
  c.prDiffEnsure(rec(), w.read);
  c.prdGoto('src/a.rs', 3, { place: 'read' });
  assert.ok(run('prDiff.target'));
  w.shown = { ...w.shown, id: 'acme/widget#2' };
  await ok(w, run, 0);
  c.prdApplyTarget();
  assert.strictEqual(run('prDiff.target'), null);
  assert.ok(!w.calls.some(x => x[0] === 'rgo'));
});

test('the draft buttons of the diff start a draft in a task\'s dialog: the column is where it appears, so no tab is switched', async () => {
  const { w, c, run } = readWorld();
  c.cardDialog.mode = 'own-read';
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  assert.strictEqual(c.prdAct(onLine(c, { prdPlus: 'right:3' }, { hit: '[data-prd-plus]' }), { place: 'dialog' }), true);
  assert.strictEqual(w.edits.length, 1);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.edits[0])), { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: '' });
  assert.ok(!w.calls.some(x => x[0] === 'gopane'));
  // The new box takes the keyboard in the dialog's column.
  assert.deepStrictEqual(JSON.parse(JSON.stringify(w.calls.find(x => x[0] === 'redraw')[1])), { sel: '[data-pr-body="p1"]', inDialog: true });
  // The explanation's own button, found by the selector the real DOM would match it by.
  const btn = el({ prdAt: 'right:3' }, { closest: sel => (sel === '[data-prd-ann]' ? el({ prdAnn: '0' }) : null), matches: sel => sel === '[data-prd-todraft]' });
  const click = { target: { closest: sel => (sel.split(',').some(x => x.trim() === '[data-prd-todraft]') ? btn : null) } };
  assert.strictEqual(c.prdAct(click, { place: 'dialog' }), true);
  assert.strictEqual(w.edits.length, 2);
});

test('the diff is drawn in the dialog of a task\'s AI 下読み as it is in the review dialog', async () => {
  const { w, c, run } = readWorld();
  c.cardDialog.mode = 'own-read';
  c.cardDialog.subject = w.selected;
  w.dialogOpen = true;
  c.prDiffEnsure(rec(), w.read);
  await ok(w, run, 0);
  const p = c.prdPlace('dialog');
  assert.strictEqual(p.root, w.dialog.root);
  c.cardDialog.mode = 'card';
  assert.strictEqual(c.prdPlace('dialog'), null);
  c.cardDialog.mode = 'own-read';
  assert.ok(run('PRD_VIEWS.dialog.ready()'));
});
