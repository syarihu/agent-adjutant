/* The 「拡大」 dialog of a whole task (src/ui/card-dialog.js): what it does when the panel redraws, and the
   gate-scoped comment box of decide.js. Run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const dlgSrc = read('card-dialog.js');
const decSrc = read('decide.js');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const fnSrc = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

/* commentBox: the box of the gate answered, whichever other boxes are on the page. */
function boxWorld() {
  const gateEl = (ref, value) => {
    const box = { value };
    return { dataset: { gate: ref }, querySelector: sel => sel === '.gate-comment' ? box : null };
  };
  const root = els => ({ querySelectorAll: sel => sel === '[data-gate]' ? els : [] });
  const dialog = root([gateEl('A', 'dialog A'), gateEl('B', 'dialog B')]);
  const panel = root([gateEl('A', 'panel A'), gateEl('C', 'panel C')]);
  const posts = [];
  const c = vm.createContext({
    document: {
      querySelector: sel => sel === '#card-dialog[open]' ? dialog : sel === '#task-panel' ? panel : null,
      createDocumentFragment: () => root([]),
    },
    gateByRef: id => ({ id, wait: true, worktree: '/w' }), gateKey: g => g.id, baseOf: () => '', baseName: x => x,
    handedNote: () => '', note() {}, gateAnswered() {}, dropGate() {}, refresh: async () => {}, refreshBoards() {},
    boardApi: async (base, url, o) => { posts.push([url, JSON.parse(o.body)]); return {}; },
  });
  vm.runInContext([fnSrc(decSrc, 'commentBox'), fnSrc(decSrc, 'answer'), fnSrc(decSrc, 'closeGate')].join('\n'), c);
  return { c, posts, dialog };
}

test('commentBox is the gate\'s own: the dialog\'s box for it, else the panel\'s, never another gate\'s', () => {
  const w = boxWorld();
  const at = ref => vm.runInContext(`commentBox(${JSON.stringify(ref)})?.value ?? null`, w.c);
  assert.strictEqual(at('B'), 'dialog B');
  assert.strictEqual(at('A'), 'dialog A');
  assert.strictEqual(at('C'), 'panel C');
  assert.strictEqual(at('Z'), null);
});

test('answering gate B reads the comment typed for B, and closing a gate reads its own too', async () => {
  const w = boxWorld();
  assert.strictEqual(await vm.runInContext(`answer('approve', undefined, 'B')`, w.c), true);
  assert.strictEqual(w.posts[0][1].comment, 'dialog B');
  assert.strictEqual(await vm.runInContext(`answer('approve', undefined, 'A')`, w.c), true);
  assert.strictEqual(w.posts[1][1].comment, 'dialog A');
  await vm.runInContext(`closeGate('B')`, w.c);
  assert.strictEqual(w.posts[2][1].comment, 'dialog B');
});

/* cardDialogTaskSync against a small stand-in for the dialog it fills: the answer column holds one dock for each
   waiting gate, and the content holds nothing of the gates' controls. */
function taskWorld({ waiting = ['A'], drafts = {}, focused = false } = {}) {
  const gateEl = (ref, value = '') => {
    const ta = { value, readOnly: false, focused: 0, focus() { this.focused++; } };
    return { dataset: { gate: ref }, attrs: { 'data-gate': ref }, ta, buttons: [{ remove() {} }],
      removeAttribute(a) { delete this.attrs[a]; }, setAttribute(a, v) { this.attrs[a] = v; },
      querySelector(sel) { return sel === '.gate-comment' ? ta : null; },
      querySelectorAll(sel) { return sel === 'button' ? this.buttons : sel === 'textarea' ? [ta] : []; } };
  };
  const dockEl = (ref, value = '') => {
    const d = { ref, gate: gateEl(ref, value), attrs: { 'data-cd-dock-for': ref }, classes: new Set(), hidden: true,
      button: { dataset: { cdDock: ref }, pressed: null, setAttribute(a, v) { if (a === 'aria-pressed') this.pressed = v; } },
      get dataset() { return { cdDockFor: this.attrs['data-cd-dock-for'] }; },
      removeAttribute(a) { delete this.attrs[a]; },
      querySelector(sel) { return sel === '[data-gate]' ? ('data-gate' in this.gate.attrs ? this.gate : null) : sel === '.gate-comment' ? this.gate.ta : null; } };
    d.classList = { add: c => d.classes.add(c) };
    return d;
  };
  const col = {
    docks: [], hidden: true, draws: 0,
    querySelectorAll(sel) {
      if (sel === '[data-gate]') return this.docks.map(d => d.gate).filter(g => 'data-gate' in g.attrs);
      if (sel === '[data-cd-dock-for]') return this.docks.filter(d => 'data-cd-dock-for' in d.attrs);
      if (sel === '.cd-dock-gone') return this.docks.filter(d => d.classes.has('cd-dock-gone'));
      if (sel === '[data-cd-dock]') return this.docks.filter(d => 'data-cd-dock-for' in d.attrs).map(d => d.button);
      return [];
    },
    querySelector(sel) { return sel === '.cd-dock' ? this.docks[0] || null : null; },
    contains: el => !!el?.inCol,
    replaceChildren(...nodes) { this.draws++; this.docks = nodes.flatMap(n => n.docks || [n]); },
  };
  waiting.forEach((r, i) => { const d = dockEl(r, drafts[r] || ''); d.hidden = i > 0; col.docks.push(d); });
  const content = {
    draws: 0, dataset: {},
    querySelectorAll: () => [],
    replaceChildren() { this.draws++; },
  };
  const goneEl = { textContent: '' };
  const body = { scrollTop: 0, focused: 0, getBoundingClientRect: () => ({ top: 0 }), focus() { this.focused++; } };
  const index = { innerHTML: '', hidden: true, replaceChildren() { this.innerHTML = ''; } };
  const titleEl = { textContent: 'T' }, subEl = { textContent: '' };
  const dlg = {
    open: true, closed: false, close() { this.open = false; this.closed = true; }, handlers: {},
    addEventListener(type, f) { this.handlers[type] = f; }, querySelectorAll: sel => col.querySelectorAll(sel),
    querySelector: sel => sel === '.card-dialog-gone' ? goneEl : sel === '.card-dialog-body' ? body : sel === '.card-dialog-index' ? index
      : sel === '.card-dialog-content' ? content : sel === '.card-dialog-dock' ? col
      : sel === '#card-dialog-title' ? titleEl : sel === '#card-dialog-sub' ? subEl : null,
  };
  const world = { waiting, subject: 't1', exists: true, hidden: false, nowKnown: true, html: 'v1', groups: [], dockMark: '', title: 'T',
    sent: [], goGate: [], closed: 0 };
  const c = vm.createContext({
    selectedTaskId: 't1', tp: () => ({ hidden: world.hidden }), state: { get now() { return world.nowKnown ? 1 : null; } },
    document: { activeElement: focused ? { matches: () => true } : null },
    taskById: () => world.exists ? { id: 't1', title: world.title } : undefined, cardDialogFocusTarget: () => null, Date,
    gatesOf: () => world.waiting.map(r => ({ id: r, openedAt: r })), gateRef: g => g.id, isWaiting: g => world.waiting.includes(g.id), claimSeq: () => 1,
    cardDialogEl: () => dlg, cardDialogContent: () => content, cardDialogPanelComment: () => null,
    cardDialogTaskGroups: () => world.groups, cardDialogTaskHtml: () => ({ index: 'i', content: world.html }),
    cardDialogDockHtml: ws => `DOCK${ws.map(g => g.id).join(',')}${world.dockMark}`,
    cardDialogFill: html => ({ docks: html.startsWith('DOCK') ? world.waiting.map(r => dockEl(r)) : [] }),
    cardDialogApplyTarget() {}, cardDialogSpy() {}, cardDialogCardEl: () => null,
    decideAct: async b => { world.sent.push(b.dataset.act); world.waiting = world.waiting.filter(r => r !== b.ref); return true; },
    gateByRef: ref => ({ id: ref, openedAt: ref }), commentBox: ref => col.docks.find(d => d.ref === ref)?.gate.ta || null,
    closeCardDialog: () => { world.closed++; }, secsStamp: secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+/, ''), cardDialogGoGate: ref => { world.goGate.push(ref); },
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};\nconst CARD_DIALOG_GONE = [^\n]*;\nconst CARD_DIALOG_GATE_GONE = [^\n]*;\nconst CARD_DIALOG_TASK_GONE = [^\n]*;\nconst CARD_DIALOG_NONE_LEFT = [^\n]*;\nconst cardDialogDrafts = [^\n]*;/m),
    cut(dlgSrc, /^const cardDialogDockEl = [^\n]*;/m),
    ...['cardDialogStrip', 'cardDialogStripEl', 'cardDialogNotice', 'cardDialogNoticeText', 'cardDialogTaskSync', 'cardDialogDockGate',
      'cardDialogFocusOf', 'cardDialogFocusBack', 'cardDialogShowDock', 'cardDialogSetDock', 'cardDialogWithAnswered',
      'cardDialogAnswer', 'cardDialogAfterAnswer', 'cardDialogSync', 'cardDialogGateButton', 'cardDialogFollowDock'].map(n => fnSrc(dlgSrc, n)),
    cut(dlgSrc, /^const cardDialogWaiting = [^\n]*;/m),
    fnSrc(dlgSrc, 'cardDialogShell'),
    cut(dlgSrc, /^cardDialogEl\(\)\.addEventListener\('close', \(\) => \{[\s\S]*?\n\}\);/m),
    `Object.assign(cardDialog, { mode: 'task', subject: 't1', dock: ${JSON.stringify(waiting[0] || null)} });`,
  ].join('\n'), c);
  const sync = (...a) => { c.args = a; return vm.runInContext('cardDialogTaskSync(...args)', c); };
  // What the opening drew is settled, so that a later sync shows only what it changes.
  if (!focused) { sync(); content.draws = 0; col.draws = 0; }
  const dockOf = ref => col.docks.find(d => d.ref === ref);
  const click = (ref, act, choice) => {
    c.b = { ref, closest: () => ({ dataset: { gate: ref } }), matches: sel => !!choice && sel === '.pick[data-choice]', dataset: { act, choice } };
    return vm.runInContext('cardDialogAnswer(b)', c);
  };
  return { c, dlg, goneEl, content, col, body, world, titleEl, index, sync, dockOf, click, state: () => vm.runInContext('cardDialog', c) };
}

test('a redraw carries what was typed for each gate, in the dock shown and in the ones that are not', () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'draft A', B: 'draft B' } });
  w.world.html = 'v2';
  w.sync();
  assert.strictEqual(w.content.draws, 1);
  // The column is drawn from the gates, and the content only changed.
  assert.strictEqual(w.col.draws, 0);
  w.world.dockMark = '!';
  w.sync();
  assert.strictEqual(w.col.draws, 1);
  assert.strictEqual(w.dockOf('A').gate.ta.value, 'draft A');
  assert.strictEqual(w.dockOf('B').gate.ta.value, 'draft B');
  // The same markup again is not drawn again.
  w.sync();
  assert.strictEqual(w.content.draws, 1);
  assert.strictEqual(w.col.draws, 1);
});

test('while a comment is typed the redraw is held, and drawn once the focus has left', () => {
  const w = taskWorld({ drafts: { A: 'half' }, focused: true });
  w.world.html = 'v2';
  w.sync();
  assert.strictEqual(w.state().held, true);
  assert.strictEqual(w.content.draws, 0);
  w.c.document.activeElement = null;
  w.sync();
  assert.strictEqual(w.state().held, false);
  assert.strictEqual(w.content.draws, 1);
  assert.strictEqual(w.dockOf('A').gate.ta.value, 'half');
});

test('a task the panel no longer has is stripped, read-only, and the dialog stays open with its notice', () => {
  const w = taskWorld({ drafts: { A: 'mine' } });
  w.world.exists = false;
  w.world.subject = 'x';
  w.sync();
  assert.match(w.goneEl.textContent, /^パネルからこのタスク/);
  assert.strictEqual(w.dlg.closed, false);
  const gate = w.dockOf('A').gate;
  assert.strictEqual(gate.attrs['data-gate'], undefined);
  assert.strictEqual(gate.ta.readOnly, true);
  assert.strictEqual(gate.ta.value, 'mine');
  assert.strictEqual(w.content.draws, 0);
  assert.strictEqual(w.state().dockSig, '');
  // Until the board has answered once, what it lacks is not known to be missing.
  const early = taskWorld();
  early.world.exists = false; early.world.nowKnown = false;
  early.sync();
  assert.strictEqual(early.goneEl.textContent, '');
});

test('another subject or a hidden panel closes the dialog quietly', () => {
  for (const o of [{ selectedTaskId: 'other' }, { hidden: true }]) {
    const w = taskWorld();
    if (o.selectedTaskId) w.c.selectedTaskId = o.selectedTaskId;
    if (o.hidden) w.world.hidden = true;
    w.sync();
    assert.strictEqual(w.dlg.closed, true, JSON.stringify(o));
    assert.strictEqual(w.state().quiet, true);
  }
});

test('a gate answered elsewhere: with a draft its dock stays read-only, shown or not; without one the column moves on', () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'unsent', B: 'for B' } });
  w.world.waiting = ['B'];
  w.sync();
  assert.match(w.goneEl.textContent, /^この gate への答え/);
  assert.strictEqual(w.state().dock, 'B');
  const [kept, fresh] = w.col.docks;
  assert.strictEqual(kept.ref, 'A');
  assert.strictEqual(kept.gate.attrs['data-gate'], undefined);
  assert.strictEqual(kept.attrs['data-cd-dock-for'], undefined);
  assert.strictEqual(kept.hidden, false);
  assert.strictEqual(kept.gate.ta.readOnly, true);
  assert.strictEqual(kept.gate.ta.value, 'unsent');
  assert.strictEqual(fresh.ref, 'B');
  assert.strictEqual(fresh.hidden, false);
  assert.strictEqual(fresh.gate.ta.value, 'for B', 'the draft is of its own gate, and not given to another');
  assert.strictEqual(w.col.hidden, false);
  // A dock that was not shown keeps its draft too.
  const hid = taskWorld({ waiting: ['A', 'B'], drafts: { B: 'hidden draft' } });
  hid.world.waiting = ['A'];
  hid.sync();
  assert.match(hid.goneEl.textContent, /^この gate への答え/);
  assert.strictEqual(hid.col.docks[0].ref, 'B');
  assert.strictEqual(hid.col.docks[0].hidden, false);
  assert.strictEqual(hid.col.docks[0].gate.ta.value, 'hidden draft');
  const bare = taskWorld({ waiting: ['A', 'B'] });
  bare.world.waiting = ['B'];
  bare.sync();
  assert.strictEqual(bare.goneEl.textContent, '');
  assert.strictEqual(bare.state().dock, 'B');
  assert.strictEqual(bare.col.docks.length, 1);
});

test('switching the dock shows one dock, marks it in the switch, and keeps every draft without a redraw', () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'draft A', B: 'draft B' } });
  const at = ref => vm.runInContext(`cardDialogSetDock(${JSON.stringify(ref)})`, w.c);
  const shown = () => w.col.docks.filter(d => !d.hidden).map(d => d.ref);
  assert.deepStrictEqual(shown(), ['A']);
  assert.strictEqual(w.dockOf('A').button.pressed, 'true');
  assert.strictEqual(w.dockOf('B').button.pressed, 'false');
  at('B');
  assert.deepStrictEqual(shown(), ['B']);
  assert.strictEqual(w.state().dock, 'B');
  assert.strictEqual(w.dockOf('A').button.pressed, 'false');
  assert.strictEqual(w.dockOf('B').button.pressed, 'true');
  at('A');
  assert.deepStrictEqual(shown(), ['A']);
  assert.strictEqual(w.dockOf('A').gate.ta.value, 'draft A');
  assert.strictEqual(w.dockOf('B').gate.ta.value, 'draft B');
  assert.strictEqual(w.col.draws, 0);
  assert.strictEqual(w.content.draws, 0);
  // A gate that has no dock changes nothing.
  at('Z');
  assert.strictEqual(w.state().dock, 'A');
  // Focus in the dock that goes away moves to the box of the one that comes.
  w.c.document.activeElement = { closest: () => w.dockOf('A') };
  vm.runInContext(`cardDialogSetDock('B', true)`, w.c);
  assert.strictEqual(w.dockOf('B').gate.ta.focused, 1);
  // A switch that scrolling made does not put the keyboard into a box: focus goes to the body instead.
  vm.runInContext(`cardDialog.dock = 'A'; cardDialogShowDock()`, w.c);
  w.c.document.activeElement = { closest: () => w.dockOf('A') };
  at('B');
  assert.strictEqual(w.dockOf('B').gate.ta.focused, 1, 'not again');
  assert.strictEqual(w.body.focused, 1);
  w.c.document.activeElement = { closest: () => null };
  at('A');
  assert.strictEqual(w.dockOf('A').gate.ta.focused, 0, 'focus elsewhere stays there');
});

test('answering a gate goes on to the next waiting gate: the dialog stays, the gate is marked answered, the draft is not kept', async () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'sent with it' } });
  await w.click('A', 'close');
  assert.strictEqual(w.dlg.closed, false);
  assert.strictEqual(w.world.closed, 0);
  const answered = w.state().answered.get('A');
  assert.strictEqual(answered.decision, 'closed');
  assert.strictEqual(answered.comment, 'sent with it');
  assert.match(answered.answeredAt, /^\d{8}T\d{6}Z$/, 'the stamp the server writes');
  assert.strictEqual(w.state().dock, 'B');
  assert.strictEqual(w.state().noMore, false);
  assert.deepStrictEqual(w.world.goGate, ['B']);
  assert.strictEqual(w.col.docks[0].gate.ta.focused, 1, 'keyboard on the next gate\'s comment');
  // Its own answer is not a gate answered elsewhere.
  assert.strictEqual(w.goneEl.textContent, '');
  assert.deepStrictEqual(w.col.docks.map(d => d.ref), ['B']);
  assert.strictEqual(w.col.docks[0].hidden, false);
  assert.strictEqual(w.state().answering, false);
  // A pick records the design it picked.
  const p = taskWorld({ waiting: ['A', 'B'] });
  await p.click('B', 'x', 'c2');
  assert.deepStrictEqual([p.state().answered.get('B').decision, p.state().answered.get('B').choice], ['choice', 'c2']);
  assert.strictEqual(p.state().dock, 'A');
});

test('answering the last waiting gate says nothing waits any more, takes the column away, and keeps the dialog', async () => {
  const w = taskWorld({ waiting: ['A'], drafts: { A: 'typed' } });
  await w.click('A', 'approve');
  assert.strictEqual(w.dlg.closed, false);
  assert.strictEqual(w.world.closed, 0);
  assert.strictEqual(w.state().noMore, true);
  assert.strictEqual(w.state().dock, null);
  assert.strictEqual(w.goneEl.textContent, '判断待ちはもうありません');
  assert.strictEqual(w.col.docks.length, 0);
  assert.strictEqual(w.col.hidden, true);
  assert.strictEqual(w.body.focused, 1);
  assert.deepStrictEqual(w.world.goGate, []);
  // Another gate opening later brings the column back and clears the notice.
  w.world.waiting = ['C'];
  w.sync();
  assert.strictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.state().noMore, false);
  assert.strictEqual(w.col.hidden, false);
  assert.deepStrictEqual(w.col.docks.map(d => d.ref), ['C']);
});

test('answering while a comment is typed for another gate still moves the column on, and keeps that comment', async () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { B: 'typing' } });
  w.c.document.activeElement = { matches: sel => sel === '#card-dialog .gate-comment' };
  await w.click('A', 'approve');
  assert.strictEqual(w.state().dock, 'B');
  assert.deepStrictEqual(w.col.docks.map(d => d.ref), ['B']);
  assert.strictEqual(w.col.docks[0].hidden, false);
  assert.strictEqual(w.dockOf('B').gate.ta.value, 'typing');
  assert.strictEqual(w.state().held, false);
});

test('answering goes to the gate after the answered one, else the first; a closed dialog is left alone', async () => {
  const w = taskWorld({ waiting: ['A', 'B', 'C'] });
  await w.click('B', 'approve');
  assert.strictEqual(w.state().dock, 'C');
  assert.deepStrictEqual(w.world.goGate, ['C']);
  await w.click('C', 'approve');
  assert.strictEqual(w.state().dock, 'A', 'none after it: from the top');
  // The sync closed the dialog (another task on screen): no jump, no focus.
  const x = taskWorld({ waiting: ['A', 'B'] });
  x.c.decideAct = async b => { x.world.waiting = x.world.waiting.filter(r => r !== b.ref); x.dlg.open = false; return true; };
  await x.click('A', 'approve');
  assert.deepStrictEqual(x.world.goGate, []);
  assert.strictEqual(x.body.focused, 0);
});

test('a pick while an answer is on its way does nothing, and otherwise switches the dock to its gate first', async () => {
  const w = taskWorld({ waiting: ['A', 'B'] });
  const pick = ref => ({ ref, closest: () => ({ dataset: { gate: ref } }), matches: sel => sel === '.pick[data-choice]', dataset: { choice: 'c1' } });
  vm.runInContext('cardDialog.answering = true', w.c);
  w.c.b = pick('B');
  await vm.runInContext('cardDialogGateButton(b)', w.c);
  assert.strictEqual(w.state().dock, 'A');
  assert.deepStrictEqual(w.world.sent, []);
  vm.runInContext('cardDialog.answering = false', w.c);
  await vm.runInContext('cardDialogGateButton(b)', w.c);
  assert.deepStrictEqual(w.world.sent, [undefined]);
  assert.strictEqual(w.state().answered.get('B').choice, 'c1');
});

test('「処理したら次へ」 does not leave the task while its dialog is open', () => {
  const myWork = cut(read('my-work.js'), /^function workAdvanceAfter\(key\) \{[\s\S]*?\n\}/m);
  const opened = [];
  const c = vm.createContext({
    prefs: { reviewNext: true }, view: 'work', work: { doc: {}, entries: new Map(), newOrder: [] },
    workSelectedId: () => 'row', workSelected: () => null, answeredGates: new Map(),
    workJudge: () => new Map(), workNextNew: () => 'next', workListRows: () => ({ rows: [{ id: 'next' }], turns: [] }),
    selectWorkRow: r => opened.push(r.id), cardDialogTaskOpen: () => false,
  });
  c.work.entries.set('row', { items: [{ kind: 'gate', key: 'k' }] });
  vm.runInContext(myWork, c);
  vm.runInContext("workAdvanceAfter('k')", c);
  assert.deepStrictEqual(opened, ['next']);
  c.cardDialogTaskOpen = () => true;
  vm.runInContext("workAdvanceAfter('k')", c);
  assert.deepStrictEqual(opened, ['next']);
  // The predicate is the dialog's own: open, and showing a task.
  const d = vm.createContext({ cardDialogEl: () => ({ open: true }) });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), fnSrc(dlgSrc, 'cardDialogTaskOpen')].join('\n'), d);
  assert.strictEqual(vm.runInContext('cardDialogTaskOpen()', d), false, 'a card alone');
  vm.runInContext("cardDialog.mode = 'task'", d);
  assert.strictEqual(vm.runInContext('cardDialogTaskOpen()', d), true);
});

test('the dock falling back to the first waiting gate follows the group in view', () => {
  const w = taskWorld({ waiting: ['A', 'B', 'C'] });
  w.world.groups = [{ key: 'waiting:C', gate: 'C' }];
  vm.runInContext(`cardDialog.active = 'waiting:C'`, w.c);
  w.world.waiting = ['B', 'C'];
  w.sync();
  assert.strictEqual(w.state().dock, 'C');
  assert.deepStrictEqual(w.col.docks.filter(d => !d.hidden).map(d => d.ref), ['C']);
  // Reading a group that is not a waiting gate's: the fallback stays.
  const x = taskWorld({ waiting: ['A', 'B'] });
  x.world.groups = [{ key: 'plan' }];
  vm.runInContext(`cardDialog.active = 'plan'`, x.c);
  x.world.waiting = ['B'];
  x.sync();
  assert.strictEqual(x.state().dock, 'B');
});

test('the history\'s record of the gate answered here does not make its sent comment a gate answered elsewhere', async () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'sent' } });
  // The history already holds A's record by the time the answer returns.
  w.c.gatesOf = () => [...w.world.waiting.map(r => ({ id: r, openedAt: r })), { id: 'A', openedAt: 'A', decision: 'approve' }];
  await w.click('A', 'approve');
  assert.strictEqual(w.goneEl.textContent, '');
  assert.deepStrictEqual(w.col.docks.map(d => d.ref), ['B']);
  assert.strictEqual(w.state().answered.size, 0, 'the record took over');
  assert.strictEqual(w.state().answeredHere.has('A'), true);
});

test('drafts typed in the column are kept by gate ref past a close, whichever task is on screen, and seed the next opening', () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'in the panel too', B: 'only here' } });
  // Another task is on screen by the time it closes: nothing is written into the panel, the drafts are still kept.
  w.c.selectedTaskId = 'other';
  w.dlg.close(); w.dlg.handlers.close();
  const keys = ctx => JSON.parse(vm.runInContext('JSON.stringify([...cardDialogDrafts])', ctx));
  assert.deepStrictEqual(keys(w.c), [['A', 'in the panel too'], ['B', 'only here']]);
  // The next opening is another task's: its own gates are B (kept), C (the panel has a cleared box) and A (no longer waits).
  const next = taskWorld({ waiting: ['B', 'C'], focused: true });
  next.c.gatesOf = () => ['A', 'B', 'C'].map(r => ({ id: r, openedAt: r }));
  next.c.cardDialogPanelComment = ref => ref === 'C' ? { value: '' } : null;
  vm.runInContext("cardDialogDrafts.set('A', 'stale'); cardDialogDrafts.set('B', 'only here'); cardDialogDrafts.set('C', 'cleared on purpose'); cardDialogDrafts.set('other/X', 'of another task');", next.c);
  next.dlg.open = false;
  vm.runInContext(`Object.assign(cardDialog, { dock: 'B' })`, next.c);
  next.col.docks = [];
  next.c.document.activeElement = null;
  next.sync(true, true);
  assert.strictEqual(next.dockOf('B').gate.ta.value, 'only here');
  assert.strictEqual(next.dockOf('C').gate.ta.value, '', 'the panel\'s box wins, even empty');
  // A (this task's, not waiting) is let go; another task's draft stays.
  assert.deepStrictEqual(keys(next.c).map(e => e[0]), ['B', 'C', 'other/X']);
});

test('a failed answer keeps the dialog, the dock and the draft; in a card alone a sent answer still closes', async () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'keep' } });
  w.c.decideAct = async () => false;
  await w.click('A', 'approve');
  assert.strictEqual(w.world.closed, 0);
  assert.strictEqual(w.state().answered.size, 0);
  assert.strictEqual(w.state().dock, 'A');
  assert.strictEqual(w.dockOf('A').gate.ta.value, 'keep');
  const card = taskWorld({ waiting: ['A'] });
  vm.runInContext(`cardDialog.mode = 'card'`, card.c);
  await card.click('A', 'approve');
  assert.strictEqual(card.world.closed, 1);
  assert.strictEqual(card.state().answered.size, 0);
});

test('an answered gate comes into the gates until the history has its record', () => {
  const answered = new Map([['A', { id: 'A', openedAt: '2026-10-02T00:00:00Z', decision: 'approve' }]]);
  const c = vm.createContext({ gateRef: g => g.id, isWaiting: () => false, claimSeq: () => 1 });
  vm.runInContext(fnSrc(dlgSrc, 'cardDialogWithAnswered'), c);
  c.answered = answered;
  const of = all => { c.all = all; return JSON.parse(JSON.stringify(vm.runInContext('cardDialogWithAnswered(all, answered)', c))).map(g => g.id + (g.decision ? ':' + g.decision : '')); };
  const old = { id: 'O', openedAt: '2026-10-01T00:00:00Z' }, late = { id: 'L', openedAt: '2026-10-03T00:00:00Z' };
  assert.deepStrictEqual(of([late, old]), ['O', 'A:approve', 'L'], 'placed by when it was opened');
  assert.strictEqual(answered.size, 1);
  // The record from the history has the decision: it wins, and the copy is let go.
  assert.deepStrictEqual(of([old, { id: 'A', openedAt: '2026-10-02T00:00:00Z', decision: 'changes' }]), ['O', 'A:changes']);
  assert.strictEqual(answered.size, 0);
  // Without a decision and no longer waiting, the copy stands in for it.
  answered.set('A', { id: 'A', openedAt: '2026-10-02T00:00:00Z', decision: 'approve' });
  assert.deepStrictEqual(of([old, { id: 'A', openedAt: '2026-10-02T00:00:00Z' }]), ['O', 'A:approve']);
});

test('a sync while an answer is on its way does nothing', () => {
  const w = taskWorld();
  w.world.html = 'v2';
  vm.runInContext('cardDialog.answering = true', w.c);
  w.sync();
  assert.strictEqual(w.content.draws, 0);
});

test('focus in the dialog is found again after the content is drawn anew', () => {
  const el = (matches, dataset, extra = {}) => ({ matches: sel => matches.includes(sel), dataset, focused: 0, focus() { this.focused++; }, closest: () => null, ...extra });
  const gate = { dataset: { gate: 'G' } };
  const btnA = el(['button'], { act: 'approve' }, { closest: sel => sel === '[data-gate]' ? gate : null });
  const btnB = el(['button'], { act: 'changes' }, { closest: sel => sel === '[data-gate]' ? gate : null });
  const box = el(['.gate-comment'], {}, { closest: sel => sel === '[data-gate]' ? gate : null });
  gate.querySelectorAll = sel => sel === 'button' ? [btnA, btnB] : sel === '.gate-comment' ? [box] : [];
  const go = el(['[data-cd-go-group]'], { cdGoGroup: 'plan', cdGoCard: 'plan:goal' });
  const goOther = el(['[data-cd-go-group]'], { cdGoGroup: 'plan' });
  const card = el(['[data-cd-card]'], { cdCard: 'plan:goal' });
  const dlg = { open: true, contains: () => true, querySelectorAll: sel => sel === '.card-dialog-index [data-cd-go-group]' ? [goOther, go]
    : sel === '[data-cd-card]' ? [card] : sel === '[data-gate]' ? [gate] : [] };
  const c = vm.createContext({ document: { activeElement: null } });
  vm.runInContext(['cardDialogFocusOf', 'cardDialogFocusBack'].map(n => fnSrc(dlgSrc, n)).join('\n'), c);
  const roundTrip = active => {
    c.document.activeElement = active; c.dlg = dlg;
    const d = vm.runInContext('cardDialogFocusOf(dlg)', c);
    c.d = d; vm.runInContext('cardDialogFocusBack(dlg, d)', c);
  };
  roundTrip(go); assert.strictEqual(go.focused, 1); assert.strictEqual(goOther.focused, 0);
  roundTrip(card); assert.strictEqual(card.focused, 1);
  roundTrip(btnB); assert.strictEqual(btnB.focused, 1); assert.strictEqual(btnA.focused, 0);
  roundTrip(box); assert.strictEqual(box.focused, 1);
  // The column's switch and 「この gate へ」 sit outside the gate's controls and are found by their gate.
  const sw = el(['[data-cd-dock]'], { cdDock: 'G2' }), swOther = el(['[data-cd-dock]'], { cdDock: 'G' });
  const goto = el(['.cd-dock-head [data-cd-goto]'], { cdGoto: 'G2' }), gotoOther = el(['.cd-dock-head [data-cd-goto]'], { cdGoto: 'G' });
  const sel = dlg.querySelectorAll;
  dlg.querySelectorAll = s => s === '[data-cd-dock]' ? [swOther, sw] : s === '.cd-dock-head [data-cd-goto]' ? [gotoOther, goto] : sel(s);
  roundTrip(sw); assert.strictEqual(sw.focused, 1); assert.strictEqual(swOther.focused, 0);
  roundTrip(goto); assert.strictEqual(goto.focused, 1); assert.strictEqual(gotoOther.focused, 0);
  // Focus outside the dialog is left alone.
  dlg.contains = () => false;
  roundTrip(btnA); assert.strictEqual(btnA.focused, 0);
});

test('closed from the head button, focus returns to the head button', () => {
  const head = { isConnected: true, getClientRects: () => [1], focus() {} };
  const c = vm.createContext({ document: { querySelectorAll: () => [], querySelector: sel => sel.includes('data-expand-task') ? head : null } });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), cut(dlgSrc, /^const cardDialogPanelCards = [^\n]*;/m), fnSrc(dlgSrc, 'cardDialogFocusTarget')].join('\n'), c);
  vm.runInContext('cardDialog.opener = null; cardDialog.section = null; cardDialog.gate = null', c);
  assert.strictEqual(vm.runInContext('cardDialogFocusTarget()', c), head);
  vm.runInContext("cardDialog.section = 'facts'", c);
  assert.strictEqual(vm.runInContext('cardDialogFocusTarget()', c), null);
});

test('what a closed dialog drew does not come back in the next opening', () => {
  const w = taskWorld({ waiting: ['A', 'B'], drafts: { A: 'unsent' } });
  w.world.waiting = ['B'];
  w.sync();
  assert.strictEqual(w.col.docks.length, 2, 'the stripped dock is kept while open');
  assert.notStrictEqual(w.goneEl.textContent, '');
  vm.runInContext(`cardDialog.answered.set('Z', {}); cardDialog.noMore = true`, w.c);
  w.dlg.close();
  w.dlg.handlers.close();
  assert.strictEqual(w.col.docks.length, 0);
  assert.strictEqual(w.col.hidden, true);
  assert.strictEqual(w.goneEl.textContent, '');
  assert.deepStrictEqual([w.state().answered.size, w.state().noMore, w.state().dockSig], [0, false, '']);
  // Another task opened: its first fill carries nothing over.
  w.dlg.open = false; w.dlg.closed = false;
  vm.runInContext(`cardDialogShell('task', 'Other', 'タスク全体'); Object.assign(cardDialog, { mode: 'task', subject: 't1', dock: 'B' });`, w.c);
  w.sync(true, true);
  assert.ok(w.col.docks.every(d => d.classes.size === 0));
  assert.strictEqual(w.goneEl.textContent, '');
  // The shell itself clears what an earlier opening left, in both modes: the column is hidden and empty.
  for (const mode of ['task', 'card']) {
    w.col.docks = [{ left: true }]; w.col.hidden = false;
    vm.runInContext(`cardDialogShell('${mode}', 'X', '')`, w.c);
    assert.strictEqual(w.col.docks.length, 0);
    assert.strictEqual(w.col.hidden, true);
  }
});

test('the title follows the task\'s, and is not rewritten when it has not changed', () => {
  const w = taskWorld();
  w.world.title = 'New title';
  w.sync();
  assert.strictEqual(w.titleEl.textContent, 'New title');
  let writes = 0;
  let value = 'New title';
  Object.defineProperty(w.titleEl, 'textContent', { get: () => value, set: v => { writes++; value = v; } });
  w.sync();
  assert.strictEqual(writes, 0);
});

test('a target still waiting for its data is dropped after a while, and by the first move by hand', () => {
  let now = 1000;
  const c = vm.createContext({
    Date: { now: () => now }, cardDialogEl: () => ({ open: true }), cardDialogResolveTarget: () => null,
    histories: {}, historyKey: () => 'k', baseOf: () => '', diffPending: () => false,
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), cut(dlgSrc, /^const CARD_DIALOG_TARGET_MS = [^\n]*/m),
    fnSrc(dlgSrc, 'cardDialogApplyTarget'), cut(dlgSrc, /^const cardDialogUserMoved = [^\n]*/m)].join('\n'), c);
  const pending = () => vm.runInContext('cardDialog.target !== null', c);
  const arm = () => vm.runInContext(`cardDialog.target = { section: 'diff', gate: null }; cardDialog.targetAt = ${now}`, c);
  const apply = () => vm.runInContext('cardDialogApplyTarget({ id: "t" }, [])', c);
  arm(); apply();
  assert.strictEqual(pending(), true, 'the history is not loaded yet');
  now += 9000; apply();
  assert.strictEqual(pending(), true);
  now += 2000; apply();
  assert.strictEqual(pending(), false, 'given up');
  arm(); vm.runInContext('cardDialogUserMoved()', c);
  assert.strictEqual(pending(), false, 'a scroll by hand');
});

test('focus on anything focusable in a card is found again by the card and its place among the focusable', () => {
  const mk = () => ({ focused: 0, focus() { this.focused++; } });
  const box = Object.assign(mk(), { matches: () => false, dataset: {} });
  const link = Object.assign(mk(), { matches: () => false, dataset: {} });
  const card = { dataset: { cdCard: 'plan:head' }, querySelectorAll: () => [box, link] };
  box.closest = sel => sel === '[data-cd-card]' ? card : null;
  link.closest = box.closest;
  const dlg = { open: true, contains: () => true, querySelectorAll: sel => sel === '[data-cd-card]' ? [card] : [] };
  const c = vm.createContext({ document: { activeElement: link }, dlg });
  vm.runInContext([cut(dlgSrc, /^const CARD_DIALOG_FOCUSABLE = [^\n]*/m), ...['cardDialogFocusOf', 'cardDialogFocusBack'].map(n => fnSrc(dlgSrc, n))].join('\n'), c);
  const d = vm.runInContext('cardDialogFocusOf(dlg)', c);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(d)), { kind: 'in', key: 'plan:head', at: 1 });
  c.d = d; vm.runInContext('cardDialogFocusBack(dlg, d)', c);
  assert.strictEqual(link.focused, 1);
  assert.strictEqual(box.focused, 0);
});

test('the index keeps the group that was clicked until the person scrolls', () => {
  const marks = {};
  const section = key => ({ dataset: { cdGroup: key }, getBoundingClientRect: () => ({ top: { a: 0, b: 400, c: 900 }[key] }) });
  const button = key => ({ dataset: { cdGoGroup: key }, hasAttribute: () => false,
    setAttribute() { marks[key] = true; }, removeAttribute() { marks[key] = false; }, scrollIntoView() {} });
  const body = { scrollTop: 800, clientHeight: 300, scrollHeight: 1100, getBoundingClientRect: () => ({ top: 0 }) };
  const followed = [];
  const c = vm.createContext({
    cardDialogEl: () => ({ querySelector: () => body, querySelectorAll: () => ['a', 'b', 'c'].map(button) }),
    cardDialogContent: () => ({ querySelectorAll: () => ['a', 'b', 'c'].map(section) }),
    cardDialogFollowDock: k => followed.push(k),
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), ...['cardDialogActiveGroup', 'cardDialogSpy'].map(n => fnSrc(dlgSrc, n)),
    cut(dlgSrc, /^const cardDialogUserMoved = [^\n]*/m), "cardDialog.mode = 'task';"].join('\n'), c);
  const spy = () => vm.runInContext('cardDialogSpy()', c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'c', 'at the end, the last group');
  vm.runInContext("cardDialog.pinned = 'b'", c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'b', 'the clicked group');
  assert.strictEqual(marks.b, true);
  vm.runInContext('cardDialogUserMoved()', c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'c');
  // The column is asked to follow only when the group changes.
  assert.deepStrictEqual(followed, ['c', 'b', 'c']);
  spy();
  assert.deepStrictEqual(followed, ['c', 'b', 'c']);
});

test('the column follows into another waiting gate\'s group, and not while typing, answering, or in the same group', () => {
  const set = [];
  const c = vm.createContext({
    document: { activeElement: null },
        cardDialogSetDock: ref => set.push(ref),
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), fnSrc(dlgSrc, 'cardDialogFollowDock'),
    "cardDialog.groups = [{ key: 'waiting:A', gate: 'A' }, { key: 'waiting:B', gate: 'B' }, { key: 'plan' }]; cardDialog.dock = 'A';"].join('\n'), c);
  const follow = key => vm.runInContext(`cardDialogFollowDock(${JSON.stringify(key)})`, c);
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B']);
  // A group that is not a waiting gate's, or the gate already shown, changes nothing: a switch by hand holds.
  follow('plan'); follow('nothing'); follow('waiting:A');
  assert.deepStrictEqual(set, ['B']);
  // Focus on a button of the column (the switch, 「この gate へ」) does not stop it: only a comment being typed does.
  vm.runInContext("cardDialog.dock = 'A'", c);
  c.document.activeElement = { matches: () => false };
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B', 'B'], 'focus on a button of the column');
  vm.runInContext("cardDialog.dock = 'A'", c);
  c.document.activeElement = { matches: sel => sel === '.card-dialog-dock .gate-comment', value: '' };
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B', 'B', 'B'], 'a box with nothing typed in it (the one an answer focused)');
  vm.runInContext("cardDialog.dock = 'A'", c);
  c.document.activeElement.value = 'text';
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B', 'B', 'B'], 'a comment typed in the column');
  c.document.activeElement = null;
  // While the card the dialog opened at is still to be found, nothing follows.
  vm.runInContext("cardDialog.dock = 'A'; cardDialog.target = { section: 'diff', gate: 'B' }", c);
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B', 'B', 'B'], 'target not drawn yet');
  vm.runInContext('cardDialog.target = null', c);
  vm.runInContext('cardDialog.answering = true', c);
  follow('waiting:B');
  assert.deepStrictEqual(set, ['B', 'B', 'B'], 'while an answer is on its way');
});

test('the pinned group is let go when the body scrolls away from where the jump left it', () => {
  const handlers = {};
  const body = { scrollTop: 500, addEventListener: (t, f) => { handlers[t] = f; } };
  const c = vm.createContext({
    cardDialogEl: () => ({ querySelector: () => body }), requestAnimationFrame: f => f(), cardDialogSpy() {},
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), cut(dlgSrc, /^const cardDialogUserMoved = [^\n]*/m),
    cut(dlgSrc, /^for \(const type of \['wheel'[^\n]*\n/m),
    cut(dlgSrc, /^cardDialogEl\(\)\.querySelector\('\.card-dialog-body'\)\.addEventListener\('scroll'[\s\S]*?\n\}\);/m),
    "cardDialog.mode = 'task'; cardDialog.pinned = 'b'; cardDialog.pinnedAt = 500;"].join('\n'), c);
  const pinned = () => vm.runInContext('cardDialog.pinned', c);
  const scroll = top => { body.scrollTop = top; handlers.scroll({ currentTarget: body }); };
  scroll(504);
  assert.strictEqual(pinned(), 'b', 'a few px is the jump settling');
  scroll(520);
  assert.strictEqual(pinned(), null, 'dragged away');
  // A press on the body (the scrollbar) lets it go too.
  vm.runInContext("cardDialog.pinned = 'b'", c);
  handlers.pointerdown();
  assert.strictEqual(pinned(), null);
});

test('a jump pins its group, remembers where it landed and focuses the card', () => {
  const el = { focused: 0, scrolled: 0, offsetWidth: 0, classList: { remove() {}, add() {} }, scrollIntoView() { this.scrolled++; }, focus() { this.focused++; } };
  const body = { scrollTop: 640 };
  const c = vm.createContext({
    cardDialogEl: () => ({ querySelector: () => body }), cardDialogCardEl: k => k === 'plan:goal' ? el : null, cardDialogGroupEl: () => null,
    cardDialogSpy() {}, setTimeout() {},
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), fnSrc(dlgSrc, 'cardDialogGo'),
    "cardDialog.groups = [{ key: 'plan', cards: [{ key: 'plan:goal' }] }]; cardDialog.target = { section: 'x' };"].join('\n'), c);
  assert.strictEqual(vm.runInContext("cardDialogGo('plan:goal', null, true)", c), true);
  assert.strictEqual(el.scrolled, 1);
  assert.strictEqual(el.focused, 1);
  assert.strictEqual(vm.runInContext('cardDialog.pinned', c), 'plan', 'the group of a card, when only the card is known');
  assert.strictEqual(vm.runInContext('cardDialog.pinnedAt', c), 640);
  assert.strictEqual(vm.runInContext('cardDialog.target', c), null);
  assert.strictEqual(vm.runInContext("cardDialogGo('nothing', null, false)", c), false);
});

test('timeline links become jumps inside the dialog, or plain text when the gate has no card', () => {
  const link = (id, text) => ({ dataset: { open: id }, textContent: text, attrs: {}, removeAttribute(a) { delete this.attrs[a]; },
    setAttribute(a, v) { this.attrs[a] = v; }, replaceWith(n) { this.replaced = n; } });
  const withCard = link('d-1', 'Diff'), without = link('x-9', 'Gone');
  const template = { set innerHTML(v) { this.html = v; }, content: { querySelectorAll: sel => sel === '[data-open]' ? [withCard, without] : [] } };
  const c = vm.createContext({
    document: { createElement: () => template, createTextNode: text => ({ text }) },
    CARD_DIALOG_DROP: '[data-expand-open]', cardDialogResolveTarget: (groups, t) => t.gate === 'D' ? 'gate:D:head' : null,
  });
  vm.runInContext(fnSrc(dlgSrc, 'cardDialogFill'), c);
  c.groups = [];
  const root = vm.runInContext(`cardDialogFill('<b>', groups, id => ({ 'd-1': 'D' })[id] || null)`, c);
  assert.strictEqual(root, template.content);
  assert.strictEqual(withCard.attrs['data-cd-goto'], 'D');
  assert.strictEqual(withCard.replaced, undefined);
  assert.strictEqual(without.replaced.text, 'Gone');
  assert.strictEqual(without.attrs['data-cd-goto'], undefined);
});
