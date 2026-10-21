/* Handing the comments of a task's PR to its worker (src/ui/task-read-drafts.js) and the guards that let the drafts' machinery
   (src/ui/pr-panel.js) serve a task as well as a review request, against a fake `boardApi`, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const panelSrc = read('pr-panel.js');
const readSrc = read('pr-read.js');
const cut = (src, re) => { const m = src.match(re); assert.ok(m, `not found: ${re}`); return m[0]; };
const fn = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));
const plain = x => JSON.parse(JSON.stringify(x));

const PR = 'acme/widget#12';
const SHA = 'a'.repeat(40);
const readOf = { sha: SHA, drafts: [{ path: 'src/a.rs', line: 4, side: 'right', body: 'from the reader' }], overall: { body: 'the reader\'s summary' } };
const rec = { id: PR, url: 'https://github.com/acme/widget/pull/12', ai: { sha: SHA } };

/* `shown` is what `prShown` answers: a task's read-through, a review request, or nothing (a task selected whose AI tab is not on). */
function world({ shown = 'task', stored = { id: PR, savedAt: 's', version: 1, drafts: [], overall: { body: '' } } } = {}) {
  const w = { notes: [], posts: [], hands: [], order: [], redrawn: 0, saves: [], handFail: null, handGate: null, refreshed: 0, timeline: 0, stored, shown };
  const c = vm.createContext({
    structuredClone, wk: () => null, CSS: { escape: x => x }, PR_BOX_SEL: '.pr-overall', JSON, Promise, clearTimeout, setTimeout: () => 1, console, BASE: '/b/acme',
    selectedTaskId: 't1', PR_REF: 'pr:', PR_SAVE_MS: 800, cardDialog: {},
    note: (...a) => { w.notes.push(a); }, prDraftsChanged: () => { w.redrawn++; }, prRedrawFor() {}, document: { getElementById: () => null }, renderTaskPanel() {},
    prNextKey: (doc, prefix) => { let n = 1; while (doc.drafts.some(d => d.key === `${prefix}${n}`)) n++; return `${prefix}${n}`; },
    prReadOf: () => readOf,
    prShown: () => (w.shown === 'task' ? { id: PR, rec, kind: 'task' } : w.shown === 'pr' ? { id: PR, rec, kind: 'pr' } : null),
    taskReadTask: () => ({ id: 't1', pr: 'https://github.com/acme/widget/pull/12' }), taskReadKey: t => `acme/${t.id}/${t.pr}`,
    taskRead: { key: 'acme/t1/https://github.com/acme/widget/pull/12' },
    refresh: async () => { w.refreshed++; }, taskReadRefresh: async () => { w.timeline++; }, refreshBoards() {},
    outboxNote: (done, data) => `${done}|${data.woken ? 'woken' : data.present ? 'present' : 'stopped'}`,
    boardApi: async (base, url, o) => {
      if (url === '/api/others/drafts' && o) { w.posts.push(JSON.parse(o.body)); w.order.push('save'); return {}; }
      if (url === '/api/tasks/t1/hand') {
        w.order.push('hand'); w.hands.push(JSON.parse(o.body));
        if (w.handGate) await w.handGate.promise;
        if (w.handFail) throw Object.assign(new Error(w.handFail.error), { why: w.handFail.why });
        return { gate: { id: 'g1', kind: 'diff', shelf: 'record' }, sent: ['p1', 'p2'], overall: true, at: '20261012T010203Z', present: true, woken: true };
      }
      if (!o) { if (!w.stored) throw Object.assign(new Error('no drafts'), { status: 404 }); return w.stored; }
      throw new Error(`unexpected ${url}`);
    },
  });
  const pieces = ['prView', 'prDraftsEntry', 'prDocOfStored', 'prEnsureDrafts', 'prEditDrafts', 'prScheduleSave', 'prSave', 'prShowSave', 'prEditRefused', 'prSaveState', 'prInput', 'prFocusOut', 'prSettleOverall', 'prSaveSettled']
    .map(n => (n === 'prView' ? 'const prView = { drafts: new Map() };' : n === 'prSaveState' ? cut(panelSrc, /^const prSaveState = [^\n]*;/m) : fn(panelSrc, n)));
  vm.runInContext(pieces.concat([cut(panelSrc, /^const PR_DRAFT_KEYS = [^\n]*;/m), fn(panelSrc, 'prSaveBody'), cut(panelSrc, /^const prDoc = [^\n]*;/m),
    fn(readSrc, 'prSeedDrafts'), fn(readSrc, 'prOverallShown')]).join('\n'), c);
  const handSrc = read('task-read-drafts.js');
  vm.runInContext(handSrc, c);
  const run = e => vm.runInContext(e, c);
  const tick = () => new Promise(r => setImmediate(r));
  return { w, c, run, tick };
}
const loaded = async (o) => {
  const x = world(o);
  x.run(`prEnsureDrafts('${PR}')`);
  await x.tick();
  return x;
};
const box = (value, extra = {}) => ({ value, dataset: {}, matches: sel => sel.split(',').some(x => x.trim() === '.pr-overall'), closest: () => null, ...extra });

test('typing and leaving the overall comment edit and save the task\'s drafts while its read-through is on, and do nothing while it is not', async () => {
  const on = await loaded({ shown: 'task' });
  on.run(`prEditDrafts('${PR}', null, d => { d.drafts.push({ key: 'p1', source: 'person', path: 'a', side: 'right', body: 'on a line' }); })`);
  on.c.input = { target: box('typed') };
  on.run('prInput(input)');
  assert.strictEqual(on.run(`prDoc('${PR}', null).overall.body`), 'typed');
  on.c.out = { target: box('x'), relatedTarget: null };
  on.run('prFocusOut(out)');
  await on.tick();
  assert.ok(on.w.posts.length >= 1, 'saved when the box was left');
  // A task selected, its AI tab not on: `prShown` is null, and the box is not one of this PR's.
  const off = await loaded({ shown: null });
  off.run(`prEditDrafts('${PR}', null, d => { d.drafts.push({ key: 'p1', source: 'person', path: 'a', side: 'right', body: 'kept' }); })`);
  const posts = off.w.posts.length;
  off.c.input = { target: box('typed elsewhere') };
  off.run('prInput(input)');
  assert.strictEqual(off.run(`prDoc('${PR}', null).overall.body`), '');
  off.c.out = { target: box('x'), relatedTarget: null };
  off.run('prFocusOut(out)');
  await off.tick();
  assert.strictEqual(off.w.posts.length, posts, 'nothing saved');
});

test('the save line of the send dialog is changed only for the PR on show', async () => {
  const { c, run } = await loaded({ shown: 'task' });
  const el = { textContent: '', classList: { toggle() {} } };
  c.document = { getElementById: () => ({ querySelectorAll: sel => (sel === '[data-pr-save]' ? [el] : []) }) };
  run(`prDraftsEntry('${PR}').saved = true`);
  run(`prShowSave('${PR}')`);
  assert.strictEqual(el.textContent, '保存しました');
  el.textContent = '';
  run("prShowSave('acme/widget#99')");
  assert.strictEqual(el.textContent, '', 'another PR\'s');
  c.prShown = () => null;
  run(`prShowSave('${PR}')`);
  assert.strictEqual(el.textContent, '', 'not on show');
});

test('handing over saves first, sends the PR the drafts are of, marks what was sent with the server\'s stamp and clears the overall comment', async () => {
  const { w, c, run, tick } = await loaded({ stored: null });
  c.read = readOf;
  // The reader brings no comments: the person adds a comment on the whole PR, one on a line, and the overall comment.
  run(`prEditDrafts('${PR}', read, d => { d.drafts.push({ key: 'p1', source: 'person', side: 'right', body: 'on the whole' }, { key: 'p2', source: 'person', path: 'src/a.rs', line: 4, side: 'right', body: 'on a line' }); d.overall = { body: 'Mostly fine' }; })`);
  await run('taskReadHand()');
  assert.deepStrictEqual(w.order.slice(0, 2), ['save', 'hand'], 'what the server reads is the saved file');
  assert.deepStrictEqual(plain(w.hands), [{ pr: PR }]);
  await tick();
  const doc = run(`prDoc('${PR}', read)`);
  assert.deepStrictEqual(plain(doc.drafts.map(d => [d.key, d.sentAt])), [['p1', '20261012T010203Z'], ['p2', '20261012T010203Z']]);
  assert.strictEqual(doc.overall.body, '');
  // The marks are saved at once, in the body the save sends.
  const last = w.posts[w.posts.length - 1].drafts;
  assert.deepStrictEqual(last.drafts.map(d => d.sentAt), ['20261012T010203Z', '20261012T010203Z']);
  assert.strictEqual(last.overall.body, '');
  // The outbox's note, the same words an answered gate gets, and the timeline and the task read again.
  assert.strictEqual(w.notes.length, 1);
  assert.deepStrictEqual(plain(w.notes[0]).slice(1), [false, '2 件の指示を worker の outbox に追記|woken']);
  assert.deepStrictEqual([w.refreshed, w.timeline], [1, 1]);
  assert.strictEqual(run(`prDraftsEntry('${PR}').handing`), false);
  assert.deepStrictEqual(plain(run(`prDraftsEntry('${PR}').handResult`)), { sent: ['p1', 'p2'], overall: true, present: true, woken: true });
});

test('the overall comment the dialog shows is written into the document before the hand-over, and cleared with the rest once it is sent', async () => {
  const { w, c, run } = await loaded({ stored: { id: PR, savedAt: 's', version: 1, drafts: [{ key: 'p1', source: 'person', body: 'whole' }], overall: null } });
  c.read = readOf;
  assert.deepStrictEqual(plain(run(`prOverallShown(prDoc('${PR}', read), read)`)), { text: 'the reader\'s summary', reader: true });
  await run('taskReadHand()');
  assert.deepStrictEqual(w.order.slice(0, 2), ['save', 'hand']);
  assert.deepStrictEqual(plain(w.posts[0].drafts.overall), { body: 'the reader\'s summary' });
  await new Promise(r => setImmediate(r));
  // Sent: the stored overall comment is blank now, and a blank one of the person's wins, so the reader's summary is not shown again.
  assert.deepStrictEqual(plain(run(`prOverallShown(prDoc('${PR}', read), read)`)), { text: '', reader: false });
});

test('a failure after the 200 (the note, the reads, the boards) never turns a hand-over that went through into a failed one', async () => {
  const { w, c, run } = await loaded({ stored: null });
  c.read = readOf;
  c.refresh = async () => { throw new Error('poll failed'); };
  c.refreshBoards = () => { throw new Error('boards failed'); };
  c.outboxNote = () => { throw new Error('wording failed'); };
  run(`prEditDrafts('${PR}', read, d => { d.drafts.push({ key: 'p1', source: 'person', body: 'a' }, { key: 'p2', source: 'person', body: 'b' }); })`);
  await run('taskReadHand()');
  assert.deepStrictEqual(plain(run(`prDraftsEntry('${PR}').handResult`)), { sent: ['p1', 'p2'], overall: true, present: true, woken: true });
  assert.ok(w.notes.every(n => n[1] !== true), 'no error note');
  assert.ok(run(`prDoc('${PR}', read)`).drafts.every(d => d.sentAt));
});

test('while it is on its way the drafts cannot be edited and a second click sends nothing', async () => {
  const { w, c, run, tick } = await loaded();
  c.read = readOf;
  run(`prEditDrafts('${PR}', read, () => {})`);
  let release; w.handGate = { promise: new Promise(r => { release = r; }) };
  const first = run('taskReadHand()');
  await tick(); await tick();
  assert.strictEqual(run(`prDraftsEntry('${PR}').handing`), true);
  assert.strictEqual(run(`prEditDrafts('${PR}', read, d => { d.overall = { body: 'x' }; })`), false);
  await run('taskReadHand()');
  w.handGate = null; release();
  await first;
  assert.strictEqual(w.hands.length, 1);
});

test('a refusal is said in the page\'s words, marks nothing and keeps the drafts and the comment', async () => {
  for (const [why, text] of [['no-gate', '差し戻せる差分の確認も記録も'], ['gate-waiting', '別の確認'], ['pr-changed', 'タスクの PR が変わりました'], ['busy', 'いま渡している'], ['gate-changed', 'そのあいだに答えられました']]) {
    const { w, c, run } = await loaded();
    c.read = readOf;
    run(`prEditDrafts('${PR}', read, d => { d.overall = { body: 'keep me' }; })`);
    w.handFail = { why, error: 'server words' };
    await run('taskReadHand()');
    const doc = run(`prDoc('${PR}', read)`);
    assert.ok(doc.drafts.every(d => !d.sentAt), why);
    assert.strictEqual(doc.overall.body, 'keep me');
    assert.ok(plain(w.notes[0])[2].includes(text), why);
    assert.strictEqual(plain(run(`prDraftsEntry('${PR}').handResult`)).error, plain(w.notes[0])[2]);
    assert.strictEqual(run(`prDraftsEntry('${PR}').handing`), false);
  }
  // A failure that has no `why` says what the server said.
  const { w, c, run } = await loaded();
  c.read = readOf;
  w.handFail = { why: null, error: 'cannot write the outbox' };
  await run('taskReadHand()');
  assert.strictEqual(plain(w.notes[0])[2], 'cannot write the outbox');
});

test('a save that fails before the hand-over stops it, and one that fails after it stays unsaved to be tried again', async () => {
  const { w, c, run } = await loaded();
  c.read = readOf;
  const api = c.boardApi;
  c.boardApi = async (base, url, o) => { if (url === '/api/others/drafts' && o) throw new Error('disk full'); return api(base, url, o); };
  run(`prEditDrafts('${PR}', read, () => {})`);
  await run('taskReadHand()');
  assert.deepStrictEqual(w.hands, [], 'nothing sent that is not saved');
  // The failed save is said once, as it starts to fail, and then the hand-over says it was not done.
  assert.ok(plain(w.notes[0])[0] === 'コメントの保存');
  assert.ok(plain(w.notes.at(-1))[2].includes('渡していません') && plain(w.notes.at(-1))[2].includes('disk full'));
  // Saves again fine, hands over; then the marks cannot be saved: they stay dirty.
  c.boardApi = api;
  let fail = false;
  c.boardApi = async (base, url, o) => { if (url === '/api/others/drafts' && o && fail) throw new Error('disk full'); const r = await api(base, url, o); if (url === '/api/tasks/t1/hand') fail = true; return r; };
  await run('taskReadHand()');
  await new Promise(r => setImmediate(r));
  assert.strictEqual(run(`prDraftsEntry('${PR}').dirty`), true);
  assert.ok(run(`prDoc('${PR}', read)`).drafts.every(d => d.sentAt));
});

test('nothing is handed over for a review request, or while the drafts are not in', async () => {
  const pr = await loaded({ shown: 'pr' });
  await pr.run('taskReadHand()');
  assert.deepStrictEqual(pr.w.hands, []);
  const none = world({ shown: 'task' });
  none.c.read = readOf;
  await none.run('taskReadHand()');
  assert.deepStrictEqual(none.w.hands, [], 'not loaded');
});

test('the drafts of the task\'s PR are asked for by its dialog, and drawing it again when they come is the task\'s own redraw', async () => {
  const src = read('task-read-view.js');
  const dialog = fn(src, 'ownReadDialogSync');
  assert.match(dialog, /prEnsureDrafts\(rec\.id\)/);
  // There is no column of drafts any more: the dialog is painted with none, and 「worker に渡す」 counts what a hand-over would send.
  assert.match(dialog, /prDialogPaint\(groups, ''/);
  assert.ok(!src.includes('OWN_HOOKS') && !read('task-read-drafts.js').includes('OWN_HOOKS'));
  assert.match(dialog, /prHandable\(prDoc\(rec\.id, read \|\| null\)\)\.drafts/);
  // What `prEnsureDrafts` does when the answer comes: `prDraftsChanged`, which for a task redraws the task's own (not the review request's panel),
  // the comments on the diff's lines, and the send dialog.
  const calls = [];
  const c = vm.createContext({
    prShown: () => ({ id: PR, kind: 'task' }), renderTaskPanel: () => calls.push('panel'), taskReadRedraw: () => calls.push('read'),
    PR_HOOKS: { diff: { comments: id => calls.push(['comments', id]) } }, prSendSync: id => calls.push(['send', id]),
    boardApi: async () => ({ id: PR, savedAt: 's', drafts: [] }),
  });
  vm.runInContext([fn(panelSrc, 'prRedrawFor'), fn(panelSrc, 'prDraftsChanged'), 'const prView = { drafts: new Map() };', fn(panelSrc, 'prDraftsEntry'), fn(panelSrc, 'prDocOfStored'), fn(panelSrc, 'prEnsureDrafts')].join('\n'), c);
  vm.runInContext(`prEnsureDrafts('${PR}')`, c);
  await new Promise(r => setImmediate(r));
  assert.deepStrictEqual(calls, ['read', ['comments', PR], ['send', PR]]);
});
