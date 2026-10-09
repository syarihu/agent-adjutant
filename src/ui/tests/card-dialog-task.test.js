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

/* cardDialogTaskSync against a small stand-in for the dialog it fills. */
function taskWorld({ waiting = ['A'], typedIn = 'A', typed = '', focused = false } = {}) {
  const gateEl = (ref, value = '') => {
    const ta = { value, readOnly: false };
    return { dataset: { gate: ref }, attrs: { 'data-gate': ref }, ta, buttons: [{ remove() {} }],
      removeAttribute(a) { delete this.attrs[a]; }, setAttribute(a, v) { this.attrs[a] = v; },
      querySelector(sel) { return sel === '.gate-comment' ? ta : null; },
      querySelectorAll(sel) { return sel === 'button' ? this.buttons : sel === 'textarea' ? [ta] : []; } };
  };
  const dock = el => ({ el, querySelector: sel => sel === '[data-gate-gone]' && 'data-gate-gone' in el.attrs ? el : null,
    before(k) { content.docks.splice(content.docks.indexOf(this), 0, k); } });
  const content = {
    docks: [], draws: 0,
    querySelectorAll(sel) {
      if (sel === '[data-gate]') return this.docks.map(d => d.el).filter(e => 'data-gate' in e.attrs);
      if (sel === '.cd-dock') return [...this.docks];
      return [];
    },
    dataset: {}, replaceChildren(nodes) { this.draws++; this.docks = nodes ? nodes.docks : []; },
  };
  content.docks.push(dock(gateEl(typedIn, typed)));
  const goneEl = { textContent: '' };
  const body = { scrollTop: 0, getBoundingClientRect: () => ({ top: 0 }), style: { setProperty() {}, removeProperty() {} } };
  const index = { innerHTML: '', hidden: true, replaceChildren() { this.innerHTML = ''; } };
  const titleEl = { textContent: 'T' }, subEl = { textContent: '' };
  const dlg = {
    open: true, closed: false, close() { this.open = false; this.closed = true; }, handlers: {},
    addEventListener(type, f) { this.handlers[type] = f; }, querySelectorAll: sel => content.querySelectorAll(sel),
    querySelector: sel => sel === '.card-dialog-gone' ? goneEl : sel === '.card-dialog-body' ? body : sel === '.card-dialog-index' ? index
      : sel === '.card-dialog-content' ? content : sel === '#card-dialog-title' ? titleEl : sel === '#card-dialog-sub' ? subEl : null,
  };
  const world = { waiting, subject: 't1', exists: true, hidden: false, nowKnown: true, html: 'v1', title: 'T' };
  const c = vm.createContext({
    selectedTaskId: 't1', tp: () => ({ hidden: world.hidden }), state: { get now() { return world.nowKnown ? 1 : null; } },
    document: { activeElement: focused ? { matches: () => true } : null },
    taskById: () => world.exists ? { id: 't1', title: world.title } : undefined, cardDialogFocusTarget: () => null, Date,
    gatesOf: () => world.waiting.map(r => ({ id: r })), gateRef: g => g.id, isWaiting: g => world.waiting.includes(g.id),
    cardDialogEl: () => dlg, cardDialogContent: () => content, cardDialogPanelComment: () => null,
    cardDialogTaskGroups: () => [], cardDialogTaskHtml: (g, d) => ({ index: 'i', content: `${world.html}${d?.id}` }),
    cardDialogFill: () => { const n = { docks: [dock(gateEl(world.waiting[0] || 'none'))],
      querySelector(sel) { return sel === '.cd-dock' ? this.docks[0] : null; }, append(k) { this.docks.push(k); } };
      n.docks[0].before = k => n.docks.unshift(k); return n; },
    cardDialogApplyTarget() {}, cardDialogSpy() {}, cardDialogCardEl: () => null,
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};\nconst CARD_DIALOG_GONE = [^\n]*;\nconst CARD_DIALOG_GATE_GONE = [^\n]*;\nconst CARD_DIALOG_TASK_GONE = [^\n]*;/m),
    ...['cardDialogStrip', 'cardDialogStripEl', 'cardDialogNotice', 'cardDialogTaskSync', 'cardDialogDockGate', 'cardDialogFocusOf', 'cardDialogFocusBack'].map(n => fnSrc(dlgSrc, n)),
    cut(dlgSrc, /^const cardDialogWaiting = [^\n]*;/m),
    fnSrc(dlgSrc, 'cardDialogShell'),
    cut(dlgSrc, /^cardDialogEl\(\)\.addEventListener\('close', \(\) => \{[\s\S]*?\n\}\);/m),
    `Object.assign(cardDialog, { mode: 'task', subject: 't1', dock: ${JSON.stringify(waiting[0] || null)} });`,
  ].join('\n'), c);
  const sync = (...a) => { c.args = a; return vm.runInContext('cardDialogTaskSync(...args)', c); };
  return { c, dlg, goneEl, content, world, titleEl, index, sync, state: () => vm.runInContext('cardDialog', c), box: () => content.docks[0].el.ta };
}

test('a redraw carries what was typed in the box of its own gate', () => {
  const w = taskWorld({ typed: 'draft A' });
  w.world.html = 'v2';
  w.sync();
  assert.strictEqual(w.content.draws, 1);
  assert.strictEqual(w.box().value, 'draft A');
  // The same markup again is not drawn again.
  w.sync();
  assert.strictEqual(w.content.draws, 1);
});

test('while a comment is typed the redraw is held, and drawn once the focus has left', () => {
  const w = taskWorld({ typed: 'half', focused: true });
  w.world.html = 'v2';
  w.sync();
  assert.strictEqual(w.state().held, true);
  assert.strictEqual(w.content.draws, 0);
  w.c.document.activeElement = null;
  w.sync();
  assert.strictEqual(w.state().held, false);
  assert.strictEqual(w.content.draws, 1);
  assert.strictEqual(w.box().value, 'half');
});

test('a task the panel no longer has is stripped, read-only, and the dialog stays open with its notice', () => {
  const w = taskWorld({ typed: 'mine' });
  w.world.exists = false;
  w.world.subject = 'x';
  w.sync();
  assert.match(w.goneEl.textContent, /^パネルからこのタスク/);
  assert.strictEqual(w.dlg.closed, false);
  const gate = w.content.docks[0].el;
  assert.strictEqual(gate.attrs['data-gate'], undefined);
  assert.strictEqual(gate.ta.readOnly, true);
  assert.strictEqual(gate.ta.value, 'mine');
  assert.strictEqual(w.content.draws, 0);
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

test('the gate of the answer band answered elsewhere: with a draft its band stays read-only, without one the band moves on', () => {
  const w = taskWorld({ waiting: ['A', 'B'], typed: 'unsent' });
  w.world.waiting = ['B'];
  w.sync();
  assert.match(w.goneEl.textContent, /^この gate への答え/);
  assert.strictEqual(w.state().dock, 'B');
  const [kept, fresh] = w.content.docks;
  assert.strictEqual(kept.el.dataset.gate, 'A');
  assert.strictEqual(kept.el.attrs['data-gate'], undefined);
  assert.strictEqual(kept.el.ta.readOnly, true);
  assert.strictEqual(kept.el.ta.value, 'unsent');
  assert.strictEqual(fresh.el.dataset.gate, 'B');
  assert.strictEqual(fresh.el.attrs['data-gate'], 'B');
  // The draft is not given to the gate that took over.
  assert.strictEqual(fresh.el.ta.value, '');
  const bare = taskWorld({ waiting: ['A', 'B'], typed: '' });
  bare.world.waiting = ['B'];
  bare.sync();
  assert.strictEqual(bare.goneEl.textContent, '');
  assert.strictEqual(bare.state().dock, 'B');
  assert.strictEqual(bare.content.docks.length, 1);
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
  const w = taskWorld({ waiting: ['A', 'B'], typed: 'unsent' });
  w.world.waiting = ['B'];
  w.sync();
  assert.strictEqual(w.content.docks.length, 2, 'the stripped band is kept while open');
  assert.notStrictEqual(w.goneEl.textContent, '');
  w.dlg.close();
  w.dlg.handlers.close();
  assert.strictEqual(w.content.docks.length, 0);
  assert.strictEqual(w.goneEl.textContent, '');
  // Another task opened: its first fill carries nothing over.
  w.dlg.open = false; w.dlg.closed = false;
  vm.runInContext(`cardDialogShell('task', 'Other', 'タスク全体'); Object.assign(cardDialog, { mode: 'task', subject: 't1', dock: 'B' });`, w.c);
  w.sync(true, true);
  assert.ok(w.content.docks.every(d => !d.querySelector('[data-gate-gone]')));
  assert.strictEqual(w.goneEl.textContent, '');
  // The shell itself clears what an earlier opening left.
  w.content.docks = [{ left: true }];
  vm.runInContext(`cardDialogShell('task', 'X', '')`, w.c);
  assert.strictEqual(w.content.docks.length, 0);
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

test('the index keeps the group that was clicked until the person scrolls, and the dock room is measured', () => {
  const marks = {};
  const section = key => ({ dataset: { cdGroup: key }, getBoundingClientRect: () => ({ top: { a: 0, b: 400, c: 900 }[key] }) });
  const button = key => ({ dataset: { cdGoGroup: key }, hasAttribute: () => false,
    setAttribute() { marks[key] = true; }, removeAttribute() { marks[key] = false; }, scrollIntoView() {} });
  const vars = {};
  const body = { scrollTop: 800, clientHeight: 300, scrollHeight: 1100, getBoundingClientRect: () => ({ top: 0 }), style: { setProperty: (k, v) => { vars[k] = v; } } };
  const dock = { querySelector: () => null, getBoundingClientRect: () => ({ height: 120.2 }) };
  const c = vm.createContext({
    cardDialogEl: () => ({ querySelector: () => body, querySelectorAll: () => ['a', 'b', 'c'].map(button) }),
    cardDialogContent: () => ({ querySelectorAll: sel => sel === '.cd-dock' ? [dock] : ['a', 'b', 'c'].map(section) }),
  });
  vm.runInContext([cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m), ...['cardDialogActiveGroup', 'cardDialogDockSpace', 'cardDialogSpy'].map(n => fnSrc(dlgSrc, n)),
    cut(dlgSrc, /^const cardDialogUserMoved = [^\n]*/m), "cardDialog.mode = 'task';"].join('\n'), c);
  const spy = () => vm.runInContext('cardDialogSpy()', c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'c', 'at the end, the last group');
  assert.strictEqual(vars['--cd-dock-h'], '121px');
  vm.runInContext("cardDialog.pinned = 'b'", c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'b', 'the clicked group');
  assert.strictEqual(marks.b, true);
  vm.runInContext('cardDialogUserMoved()', c);
  spy();
  assert.strictEqual(vm.runInContext('cardDialog.active', c), 'c');
  dock.querySelector = () => ({});
  spy();
  assert.strictEqual(vars['--cd-dock-h'], '0px');
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

test('the answer band is a named group, and only the last one sticks', () => {
  const src = cut(dlgSrc, /\(dockGate \? `<div class="cd-dock"[^\n]*/);
  assert.match(src, /role="group" aria-label="\$\{esc\(/);
  assert.match(fs.readFileSync(path.join(__dirname, '..', 'card-dialog.css'), 'utf8'), /\.cd-dock:has\(~ \.cd-dock\)\s*\{\s*position:\s*static/);
});
