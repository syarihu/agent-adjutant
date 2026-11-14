/* A task handed over with Stop at none (nothing waits): the words the board shows for it, the pill on the card, and the plan
   it is worked to, which is a record rather than an approved gate. Run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};

const ctx = vm.createContext({ TextEncoder, TextDecoder });
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
run(read('util.js'), 'util.js');
const board = read('board.js');
run([
  cut(board, /^const STOP_AT = [^\n]*\n/m),
  cut(board, /^function recordSummary\(r\) \{[\s\S]*?\n\}\n/m),
  cut(read('cards.js'), /^function stopAtPillHtml\(task\) \{[\s\S]*?\n\}\n/m),
].join('\n'), 'stubs');
const { STOP_AT, recordSummary, stopAtPillHtml, planOfTask } = vm.runInContext('({ STOP_AT, recordSummary, stopAtPillHtml, planOfTask })', ctx);

test('none is a stop point the board has words for, and the other three keep theirs', () => {
  assert.equal(STOP_AT.none, '承認を待たない');
  assert.ok(STOP_AT.plan && STOP_AT.diff && STOP_AT.all);
});

test('the card says 承認なし for a task that stops at none and for no other', () => {
  const html = stopAtPillHtml({ stopAt: 'none' });
  assert.ok(html.includes('承認なし') && html.includes('title="'));
  for (const stopAt of ['plan', 'diff', 'all', undefined]) assert.equal(stopAtPillHtml({ stopAt }), '');
});

test('a plan kept as a record has a neutral summary', () => {
  assert.deepEqual(recordSummary({ kind: 'plan', wait: false }), ['計画(承認なしで進めた)', '']);
});

test('the plan of a task is the approved one, else the latest plan record', () => {
  const approved = { id: 'a', kind: 'plan', decision: 'approve' };
  const early = { id: 'r1', kind: 'plan', wait: false, openedAt: '20260101T000000Z' };
  const late = { id: 'r2', kind: 'plan', wait: false, openedAt: '20260102T000000Z' };
  const diff = { id: 'd', kind: 'diff', wait: false, openedAt: '20260103T000000Z' };
  assert.equal(planOfTask({ approvedPlan: approved, records: [late] }), approved);
  assert.equal(planOfTask({ records: [late, diff, early] }), late);
  assert.equal(planOfTask({ records: [diff] }), null);
  assert.equal(planOfTask({}), null);
  assert.equal(planOfTask(null), null);
});
