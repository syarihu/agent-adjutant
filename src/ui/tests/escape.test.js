/* Escape on the board and in 「いまの仕事」 (the top of src/ui/main.js), run with `node --test`. Escape is the
   agent's key in a session terminal and never a page shortcut that closes the selection (#667). */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'main.js'), 'utf8');
// From the first top-level piece of the file (the terminal helper, or the keydown listener) up to setView.
const head = src.match(/^[\s\S]*?(?=\nfunction setView)/);
assert.ok(head, 'top of main.js not found');
const keydownBlock = src.match(/document\.addEventListener\('keydown'[\s\S]*?\n\}\);/);
assert.ok(keydownBlock, 'keydown listener not found');

const inTerm = { closest: s => (s === '.adj-terminal' ? inTerm : null), matches: () => false };
const outside = { closest: () => null, matches: () => false };
const dlg = id => ({ id, closed: 0, close() { this.closed++; }, matches: s => s === 'dialog' });

const run = ({ target = outside, active = outside, view = 'work', selected = 't1', dialog = null, composing = false, keyCode = 27 }) => {
  const h = {};
  const calls = { panel: 0, handover: 0 };
  const ctx = vm.createContext({
    document: {
      addEventListener: (t, fn) => { h[t] = fn; },
      querySelector: s => (s === 'dialog[open]' ? dialog : null),
      activeElement: active,
    },
    view, selectedTaskId: selected,
    closeTaskPanel: () => { calls.panel++; },
    closeHandoverDialog: () => { calls.handover++; },
  });
  vm.runInContext(head[0], ctx);
  const esc = () => h.keydown({ key: 'Escape', target, isComposing: composing, keyCode });
  return { calls, h, esc };
};

test('Escape with focus on the body (work view; the middle terminal mounted without focus or remounted) does not close the selection (#667)', () => {
  const r = run({ target: outside, view: 'work' });
  r.esc();
  assert.strictEqual(r.calls.panel, 0);
});

test('Escape on the board does not close the selection', () => {
  const r = run({ view: 'board' });
  r.esc();
  assert.strictEqual(r.calls.panel, 0);
});

test('Escape on a terminal head or bar button, outside .adj-terminal, does not close the selection', () => {
  const btn = { closest: () => null, matches: s => s === 'button' };
  const r = run({ target: btn, active: btn });
  r.esc();
  assert.strictEqual(r.calls.panel, 0);
});

test('Escape in a terminal does not close the #form dialog', () => {
  const d = dlg('form');
  const r = run({ target: inTerm, active: inTerm, dialog: d });
  r.esc();
  assert.strictEqual(d.closed, 0);
  assert.strictEqual(r.calls.panel, 0);
});

test('Escape outside a terminal closes #form once and the handover dialog once', () => {
  const d = dlg('form');
  run({ dialog: d }).esc();
  assert.strictEqual(d.closed, 1);
  const r = run({ dialog: dlg('handover-dialog') });
  r.esc();
  assert.strictEqual(r.calls.handover, 1);
  assert.strictEqual(r.calls.panel, 0);
});

test('Escape during IME composition closes nothing', () => {
  for (const opts of [{ composing: true }, { keyCode: 229 }]) {
    const d = dlg('form');
    const r = run({ dialog: d, ...opts });
    r.esc();
    assert.strictEqual(d.closed, 0);
    assert.strictEqual(r.calls.handover + r.calls.panel, 0);
  }
});

test('cancel on a dialog is prevented only while a terminal has the focus', () => {
  const ev = (target) => ({ target, prevented: 0, preventDefault() { this.prevented++; } });
  const d = dlg('x');
  const inside = run({ active: inTerm });
  const e1 = ev(d); inside.h.cancel(e1);
  assert.strictEqual(e1.prevented, 1);
  const out = run({ active: outside });
  const e2 = ev(d); out.h.cancel(e2);
  assert.strictEqual(e2.prevented, 0);
  const e3 = ev(outside); inside.h.cancel(e3);
  assert.strictEqual(e3.prevented, 0);
});

test('the keydown listener has no closeTaskPanel', () => {
  assert.ok(!keydownBlock[0].includes('closeTaskPanel'));
});
