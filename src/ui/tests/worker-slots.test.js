/* The hub panel's worker count (workerSlotsText, src/ui/task-panel.js), run with `node --test`. With no maxWorkers the cap is null and
   the text says there is no limit instead of printing 「/ null」. */
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
const ctx = vm.createContext({});
vm.runInContext(`${cut(/^const workerSlotsText = [^\n]*;/m)}\nthis.workerSlotsText = workerSlotsText;`, ctx);
const text = w => ctx.workerSlotsText(w);

test('no cap reads as unlimited', () => {
  assert.strictEqual(text({ busy: 3, max: null }), '3 稼働（上限なし）');
  assert.strictEqual(text({ busy: 3 }), '3 稼働（上限なし）');
  assert.strictEqual(text({ busy: 0, max: null }), '0 稼働（上限なし）');
});

test('a cap reads as busy / max', () => {
  assert.strictEqual(text({ busy: 3, max: 5 }), '3 / 5 稼働');
  assert.strictEqual(text({ busy: 5, max: 5 }), '5 / 5 稼働');
  assert.strictEqual(text({ max: 2 }), '0 / 2 稼働');
});

test('never prints null or undefined', () => {
  for (const w of [{ busy: 3, max: null }, { busy: 3 }, { busy: 3, max: 5 }, { busy: 5, max: 5 }, { busy: 0, max: null }, { max: 2 }]) {
    assert.doesNotMatch(text(w), /null|undefined/);
  }
});
