/* The AI 下読み that backs a diff gate's card (src/ui/question-dialog.js, `questionReadOf`), against stand-ins for the page, run
   with `node --test`: what is asked for and when, what is kept, and which open card is drawn again when an answer comes. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'question-dialog.js'), 'utf8');
const SHA = 'a'.repeat(40);
const task = (id = 't1', pr = 'acme/widget#12') => ({ id, pr, title: 'Add a timer' });
const rec = (extra = {}) => ({ id: 'acme/widget#12', ai: { sha: SHA, stale: false }, ...extra });
const readOf = (extra = {}) => ({ sha: SHA, madeAt: 'm1', annotations: [{ path: 'a.rs', anchor: 'x', kind: 'ex', text: 'T' }], ...extra });
const gate = (extra = {}) => ({ id: 'g1', kind: 'diff', _slug: 'acme', _base: '/b/acme', ...extra });

function world() {
  const w = { gets: [], pending: [], clock: 1000, now: 0, drawn: [], open: true, task: task(), body: { scrollTop: 7, innerHTML: 'OLD', contains: () => false } };
  const layer = { querySelector: s => (s === '.qd-body' ? w.body : null) };
  const ctx = vm.createContext({
    console, Promise, encodeURIComponent, document: { getElementById: () => ({ addEventListener() {} }), activeElement: null },
    Date: { now: () => w.clock }, nav: { board: 'acme' }, multiBoard: true, BASE: '/b/acme', boards: [], view: 'work', state: { gates: [], now: 1 },
    TODO_BOARD_FRESH_MS: 1000, answeredGates: new Set(), manualChecked: new Set(),
    registerView() {}, dialogStack: { isOpen: () => w.open, layerEl: () => layer, has: () => false, push() {}, closeAll() {} },
    taskForGate: () => w.task, gateKey: g => `${g._slug || ''}/${g.id}`, issueNumberOf: () => '',
    questionCardHtml: (g, c) => { w.drawn.push(c.read); return { head: '', body: `BODY ${c.read ? c.read.madeAt : 'none'}`, foot: '' }; },
    boardApi: (base, url) => new Promise((resolve, reject) => { w.gets.push({ base, url }); w.pending.push({ resolve, reject }); }),
  });
  vm.runInContext(src, ctx, { filename: 'question-dialog.js' });
  w.c = ctx;
  w.run = e => vm.runInContext(e, ctx);
  return w;
}
const tick = () => new Promise(r => setImmediate(r));
const fail = (status, message = 'x') => Object.assign(new Error(message), { status });

test('only a diff gate of a task with a PR is given a read-through, and none is asked for the others', () => {
  const w = world();
  assert.strictEqual(w.c.questionReadOf(gate({ kind: 'plan' }), task()), null);
  assert.strictEqual(w.c.questionReadOf(gate(), { id: 't1' }), null);
  assert.strictEqual(w.c.questionReadOf(gate(), null), null);
  assert.strictEqual(w.gets.length, 0, 'reading asks for nothing');
  w.c.questionReadEnsure(gate({ kind: 'plan' }), task());
  w.c.questionReadEnsure(gate(), { id: 't1' });
  assert.strictEqual(w.gets.length, 0);
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.c.questionReadOf(gate(), task()), null, 'null until it has come');
  assert.deepStrictEqual(w.gets.map(x => [x.base, x.url]), [['/b/acme', '/api/tasks/t1/ai']]);
});

test('the read-through is kept only when it is of the PR\'s head now, is not a split proposal and has annotations', async () => {
  const check = async (docOf, readAnswer) => {
    const w = world();
    w.c.questionReadEnsure(gate(), task());
    docOf instanceof Error ? w.pending[0].reject(docOf) : w.pending[0].resolve(docOf);
    await tick();
    if (w.pending[1]) { readAnswer instanceof Error ? w.pending[1].reject(readAnswer) : w.pending[1].resolve(readAnswer); await tick(); }
    return { w, read: w.c.questionReadOf(gate(), task()) };
  };
  const good = await check({ record: rec() }, readOf());
  assert.strictEqual(good.read.madeAt, 'm1');
  assert.deepStrictEqual(good.w.gets.map(x => [x.base, x.url]), [['/b/acme', '/api/tasks/t1/ai'], ['', '/api/others/read?id=acme%2Fwidget%2312']]);
  // No record, no read-through, or one that is not of the head now: nothing is asked for further.
  for (const doc of [{ record: null }, {}, { record: rec({ ai: { status: 'queued' } }) }, { record: rec({ ai: { sha: SHA, stale: true } }) }]) {
    const x = await check(doc);
    assert.strictEqual(x.read, null);
    assert.strictEqual(x.w.gets.length, 1);
  }
  // The stored copy is another head's, a split proposal, or has no annotations.
  assert.strictEqual((await check({ record: rec() }, readOf({ sha: 'b'.repeat(40) }))).read, null);
  assert.strictEqual((await check({ record: rec() }, readOf({ mode: 'split' }))).read, null);
  assert.strictEqual((await check({ record: rec() }, readOf({ annotations: [] }))).read, null);
  // None stored (404) is an answer, from either call.
  assert.strictEqual((await check({ record: rec() }, fail(404))).read, null);
  assert.strictEqual((await check(fail(404))).read, null);
});

test('a failure keeps the read-through that was held and says so; it is asked again after a minute, not before', async () => {
  const w = world();
  w.c.questionReadEnsure(gate(), task());
  w.pending[0].resolve({ record: rec() });
  await tick();
  w.pending[1].resolve(readOf());
  await tick();
  const key = 'acme|t1|acme/widget#12';
  assert.strictEqual(w.c.questionReadOf(gate(), task()).madeAt, 'm1');
  assert.strictEqual(w.gets.length, 2, 'young: not asked again');
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.gets.length, 2, 'young: not asked again by ensure either');
  w.clock += 60 * 1000 + 1;
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.c.questionReadOf(gate(), task()).madeAt, 'm1', 'the old one is still given while the new is asked');
  assert.strictEqual(w.gets.length, 3);
  // While it is out it is not asked a second time.
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.gets.length, 3);
  w.pending[2].reject(fail(500, 'boom'));
  await tick();
  assert.strictEqual(w.run(`questionReads.get(${JSON.stringify(key)}).error`), 'boom');
  assert.strictEqual(w.c.questionReadOf(gate(), task()).madeAt, 'm1', 'a failure is not an empty answer');
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.gets.length, 3, 'tried again only after a minute');
  w.clock += 60 * 1000 + 1;
  w.c.questionReadEnsure(gate(), task());
  assert.strictEqual(w.gets.length, 4);
  // A good answer clears the error.
  w.pending[3].resolve({ record: rec({ ai: { sha: 'c'.repeat(40), stale: false } }) });
  await tick();
  w.pending[4].resolve(readOf({ sha: 'c'.repeat(40), madeAt: 'm2' }));
  await tick();
  assert.strictEqual(w.run(`questionReads.get(${JSON.stringify(key)}).error`), null);
  assert.strictEqual(w.c.questionReadOf(gate(), task()).madeAt, 'm2');
});

test('the board, the task and the PR make the key, and no more than eight are kept, the oldest going first', () => {
  const w = world();
  assert.strictEqual(w.run(`questionReadKey(${JSON.stringify(gate())}, ${JSON.stringify(task())})`), 'acme|t1|acme/widget#12');
  assert.strictEqual(w.run(`questionReadKey({ kind: 'diff' }, ${JSON.stringify(task())})`), 'acme|t1|acme/widget#12', 'the page\'s board when the gate has none');
  assert.notStrictEqual(w.run(`questionReadKey(${JSON.stringify(gate({ _slug: 'other' }))}, ${JSON.stringify(task())})`), 'acme|t1|acme/widget#12');
  for (let i = 0; i < 8; i++) w.c.questionReadEnsure(gate(), task(`t${i}`, `acme/widget#${i}`));
  // The answers have come (an entry whose answer is on its way is never the one to go: see below).
  w.run('questionReads.forEach(e => { e.busy = false; })');
  // A hit makes an entry the most recent, so the next to go is the one after it.
  w.c.questionReadOf(gate(), task('t0', 'acme/widget#0'));
  w.c.questionReadEnsure(gate(), task('t8', 'acme/widget#8'));
  w.c.questionReadEnsure(gate(), task('t9', 'acme/widget#9'));
  const keys = JSON.parse(w.run('JSON.stringify([...questionReads.keys()])'));
  assert.strictEqual(keys.length, 8);
  assert.deepStrictEqual(keys.slice(0, 2), ['acme|t3|acme/widget#3', 'acme|t4|acme/widget#4']);
  assert.ok(keys.includes('acme|t0|acme/widget#0') && !keys.includes('acme|t1|acme/widget#1') && !keys.includes('acme|t2|acme/widget#2'));
  // While every answer is on its way, none goes: a late answer must land in an entry that is still there.
  w.run('questionReads.forEach(e => { e.busy = true; })');
  w.c.questionReadEnsure(gate(), task('t10', 'acme/widget#10'));
  assert.strictEqual(w.run('questionReads.size'), 9);
});

/* An open card for the gate of task `t1`, as `openQuestionDialog` makes it. */
function openCard(w, { t = task(), g = gate() } = {}) {
  w.c.__base = { key: 'acme/g1', g, ctx: { task: t, read: null }, body: 'BODY none', gone: false, opts: {} };
  w.run('questionBase = __base');
  return w.c.__base;
}

test('an answer draws the open card of that task again with the read-through, keeping the scroll', async () => {
  const w = world();
  const base = openCard(w);
  w.c.questionReadEnsure(gate(), task());
  w.pending[0].resolve({ record: rec() });
  await tick();
  w.pending[1].resolve(readOf());
  await tick();
  assert.strictEqual(w.body.innerHTML, 'BODY m1');
  assert.strictEqual(w.body.scrollTop, 7);
  assert.strictEqual(base.body, 'BODY m1');
  assert.strictEqual(base.ctx.read.madeAt, 'm1');
  assert.strictEqual(base.ctx.task.id, 't1');
});

test('an answer for another task or board leaves the open card alone, and so does focus inside it', async () => {
  const w = world();
  const base = openCard(w);
  // Another task\'s answer.
  w.c.questionReadEnsure(gate(), task('t2', 'acme/widget#2'));
  w.pending[0].resolve({ record: rec({ id: 'acme/widget#2' }) });
  await tick();
  w.pending[1].resolve(readOf());
  await tick();
  assert.strictEqual(w.body.innerHTML, 'OLD');
  // The same task on another board.
  w.c.questionReadEnsure(gate({ _slug: 'other', _base: '/b/other' }), task());
  w.pending[2].resolve({ record: rec() });
  await tick();
  w.pending[3].resolve(readOf());
  await tick();
  assert.strictEqual(w.body.innerHTML, 'OLD');
  assert.strictEqual(base.body, 'BODY none');
  // Focus is inside the body: it is not replaced under the person\'s hands (the next round does it).
  w.c.document.activeElement = { inside: true };
  w.body.contains = el => el === w.c.document.activeElement;
  w.c.questionReadEnsure(gate(), task());
  w.pending[4].resolve({ record: rec() });
  await tick();
  w.pending[5].resolve(readOf());
  await tick();
  assert.strictEqual(w.body.innerHTML, 'OLD');
  assert.strictEqual(base.ctx.read, null);
  // A card that is gone is not drawn.
  w.c.document.activeElement = null;
  w.body.contains = () => false;
  base.gone = true;
  w.clock += 61 * 1000;
  w.c.questionReadEnsure(gate(), task());
  w.pending[6].resolve({ record: rec({ ai: { sha: 'c'.repeat(40), stale: false } }) });
  await tick();
  w.pending[7].resolve(readOf({ sha: 'c'.repeat(40), madeAt: 'm9' }));
  await tick();
  assert.strictEqual(w.body.innerHTML, 'OLD');
});

test('a read-through that goes away (the PR moved on) draws the card without it', async () => {
  const w = world();
  w.c.questionReadEnsure(gate(), task());
  w.pending[0].resolve({ record: rec() });
  await tick();
  w.pending[1].resolve(readOf());
  await tick();
  const base = openCard(w);
  base.ctx.read = readOf();
  base.body = 'BODY m1';
  w.body.innerHTML = 'BODY m1';
  w.clock += 61 * 1000;
  w.c.questionReadEnsure(gate(), task());
  w.pending[2].resolve({ record: rec({ ai: { sha: SHA, stale: true } }) });
  await tick();
  assert.strictEqual(w.body.innerHTML, 'BODY none');
  assert.strictEqual(base.ctx.read, null);
});

test('computing the ctx of many diff gates asks for nothing; opening one card asks for that one only', () => {
  const w = world();
  const tasks = Array.from({ length: 12 }, (_, i) => task(`t${i}`, `acme/widget#${i}`));
  const gates = tasks.map((t, i) => gate({ id: `g${i}`, task: t.id }));
  w.c.taskForGate = g => tasks.find(t => t.id === g.task);
  for (let round = 0; round < 3; round++) for (const g of gates) w.c.questionCtxOf(g);
  assert.strictEqual(w.gets.length, 0);
  assert.strictEqual(w.run('questionReads.size'), 0);
  // Opening the card of one asks for its task alone.
  w.c.__g = gates[3];
  w.run('openQuestionDialog(__g, { ctx: questionCtxOf(__g) })');
  assert.deepStrictEqual(w.gets.map(x => x.url), ['/api/tasks/t3/ai']);
  // Following the page asks nothing more while the entry is young or out.
  w.c.state.gates = [gates[3]];
  w.run('questionDialogFollow()');
  assert.strictEqual(w.gets.length, 1);
});
