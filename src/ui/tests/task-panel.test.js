/* The width handle of the task panel (src/ui/task-panel.js), run with `node --test`. The side it grows from is
   read from body's panel-right class, not prefs.panelDock: the work view keeps the panel on the right whatever
   the dock says. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'task-panel.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};

const el = (extra = {}) => {
  const on = {};
  return {
    on, offsetWidth: 0, ...extra,
    addEventListener(t, fn) { on[t] = fn; },
    removeEventListener(t, fn) { if (on[t] === fn) delete on[t]; },
    setPointerCapture() {},
  };
};
const els = { 'nav-rail': el({ offsetWidth: 56 }), 'task-panel': el({ offsetWidth: 600 }), 'tp-resize': el() };
const classes = new Set();
const saved = { n: 0 };
const ctx = vm.createContext({
  tp: id => els[id],
  document: {
    body: {
      classList: {
        contains: c => classes.has(c), add: c => classes.add(c), remove: c => classes.delete(c),
        toggle: (c, on) => (on ?? !classes.has(c)) ? classes.add(c) : classes.delete(c),
      },
      style: { props: {}, setProperty(k, v) { this.props[k] = v; } },
    },
  },
  prefs: { panelDock: 'right', panelWidth: 520 },
  innerWidth: 1600,
  panelPop: () => false,
  matchMedia: () => ({ matches: false }),
  savePrefs: () => { saved.n++; },
});
vm.runInContext(cut(/^\/\* The width is dragged[\s\S]*?\n  handle\.addEventListener\('pointercancel', end\);\n\}\);/m), ctx);

const handle = els['tp-resize'];
const setSide = (dock, right) => {
  ctx.prefs.panelDock = dock;
  if (right) classes.add('panel-right'); else classes.delete('panel-right');
};
const key = k => {
  handle.on.keydown({ key: k, preventDefault() {} });
  return ctx.prefs.panelWidth;
};
const drag = x => {
  handle.on.pointerdown({ pointerId: 1, currentTarget: handle, preventDefault() {} });
  assert.ok(classes.has('tp-dragging'));
  handle.on.pointermove({ clientX: x });
  const w = ctx.prefs.panelWidth;
  handle.on.pointerup();
  assert.ok(!classes.has('tp-dragging'));
  assert.ok(!handle.on.pointermove, 'the move listener is taken off on release');
  return w;
};

test('docked left but on the right (the work view): the handle is on the left edge', () => {
  setSide('left', true);
  assert.strictEqual(key('ArrowRight'), 576);
  assert.strictEqual(key('ArrowLeft'), 624);
  assert.strictEqual(drag(1000), 1600 - 1000);
  assert.strictEqual(ctx.document.body.style.props['--panel-w'], '600px');
});

test('docked right but without panel-right: the class wins, the handle is on the right edge', () => {
  setSide('right', false);
  assert.strictEqual(key('ArrowRight'), 624);
  assert.strictEqual(key('ArrowLeft'), 576);
  assert.strictEqual(drag(1000), 1000 - 56);
});

test('the width is saved after a key step and after a drag', () => {
  setSide('left', true);
  const before = saved.n;
  key('ArrowLeft');
  drag(900);
  assert.strictEqual(saved.n, before + 2);
});
