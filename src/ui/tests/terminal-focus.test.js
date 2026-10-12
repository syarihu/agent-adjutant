/* The note for a terminal that loses the focus to nothing (the top of src/ui/terminal.js), run with
   `node --test`. A browser extension that takes Escape blurs the text area; the page only sees that. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const block = src.match(/let termPointerAt = 0;[\s\S]*?\n\}, true\);\n(?=\n?const TERMINAL_ENDS)/);
assert.ok(block, 'focus note block not found in terminal.js');

const host = (visible = true) => ({ checkVisibility: () => visible });
const textarea = ({ host: h = host(), connected = true, cls = 'xterm-helper-textarea', inTerminal = true } = {}) => ({
  classList: { contains: c => c === cls },
  isConnected: connected,
  closest: s => (s === '.adj-terminal' && inTerminal ? h : null),
});
const outside = { closest: () => null };
const inside = { closest: s => (s === '.adj-terminal' ? host() : null) };

const setup = ({ hasFocus = true, active = outside } = {}) => {
  const h = {};
  const notes = [];
  const queue = [];
  const doc = {
    addEventListener: (t, fn) => { h[t] = fn; },
    hasFocus: () => hasFocus,
    activeElement: active,
  };
  const ctx = vm.createContext({
    document: doc,
    note: (...a) => notes.push(a),
    setTimeout: fn => queue.push(fn),
    Date: { now: () => ctx.__now },
    __now: 100000,
  });
  vm.runInContext(block[0], ctx);
  return {
    notes, doc, ctx,
    focusout: (ta, relatedTarget = null) => h.focusout({ target: ta, relatedTarget }),
    pointerdown: target => h.pointerdown({ target }),
    settle: () => { while (queue.length) queue.shift()(); },
  };
};

test('notes once when the terminal loses the focus to nothing', () => {
  const r = setup();
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 1);
  assert.strictEqual(r.notes[0][1], true);
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 1);
});

test('quiet when the text area is gone by the time the check runs (the page disposed it)', () => {
  const r = setup();
  const ta = textarea();
  r.focusout(ta);
  ta.isConnected = false;
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('quiet when the terminal is hidden by then (another tab of the panel)', () => {
  const r = setup();
  const h = host(true);
  r.focusout(textarea({ host: h }));
  h.checkVisibility = () => false;
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('quiet when the focus is inside a terminal by then (a new terminal took it)', () => {
  const r = setup();
  r.focusout(textarea());
  r.doc.activeElement = inside;
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('quiet after a press outside the terminal 100 ms before', () => {
  const r = setup();
  r.pointerdown(outside);
  r.ctx.__now += 100;
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('a press inside the terminal does not block the note', () => {
  const r = setup();
  r.pointerdown(inside);
  r.ctx.__now += 100;
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 1);
});

test('an outside press long ago does not block the note', () => {
  const r = setup();
  r.pointerdown(outside);
  r.ctx.__now += 2000;
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 1);
});

test('quiet when the focus went to another element', () => {
  const r = setup();
  r.focusout(textarea(), {});
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('quiet when the window lost the focus', () => {
  const r = setup({ hasFocus: false });
  r.focusout(textarea());
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});

test('quiet for a text area outside .adj-terminal or without the xterm class', () => {
  const r = setup();
  r.focusout(textarea({ inTerminal: false }));
  r.focusout(textarea({ cls: 'other' }));
  r.settle();
  assert.strictEqual(r.notes.length, 0);
});
