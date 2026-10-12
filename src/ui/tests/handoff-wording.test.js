/* What the places that list a gate's answers say of the points a person handed to the worker from a read-through
   (src/ui/task-view.js timeline, src/ui/decide.js, src/ui/my-work-away.js), run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cut = (src, re) => { const m = src.match(re); assert.ok(m, `not found: ${re}`); return m[0]; };
const fn = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

const ctx = vm.createContext({
  DONE_WHEN: {}, STOP_AT: {}, stopWhy: () => [], stopBad: () => false, recordSummary: () => ['ok', 'good'], stampSecs: s => Number(String(s).slice(-4)), isWaiting: () => false,
  ago: s => `ago(${s})`,
});
vm.runInContext(read('util.js'), ctx, { filename: 'util.js' });
vm.runInContext(fn(read('task-view.js'), 'historyEventsOf'), ctx);
const handoff = { pr: 'acme/widget#7', sha: 'a'.repeat(40), count: 3, overall: true };
const task = { id: 't1', createdAt: '20261012T0000', doneWhen: 'pr' };
const rec = (extra = {}) => ({ id: 'g-record', kind: 'diff', title: 'Round the amounts', wait: false, openedAt: '20261012T0001', answers: [], ...extra });
const events = gates => vm.runInContext('historyEventsOf', ctx)(task, gates).map(e => e.html);

test('an answer of a record with the read-through\'s points says how many went to the worker, and folds what was sent', () => {
  const [, , sent] = events([rec({ answers: [{ decision: 'changes', comment: '### 1. a.rs:4 (AI)\n<b>x</b>', answeredAt: '20261012T0005', handoff }] })]);
  assert.ok(sent.includes('<div>AI 下読みの指示 3 件を worker に渡した・全体のコメントも — コードレビューの記録「Round the amounts」</div>'), sent);
  assert.ok(sent.includes('<details class="handoff"><summary>渡した内容</summary>') && sent.includes('&lt;b&gt;x&lt;/b&gt;') && !sent.includes('<b>x'));
  assert.ok(!sent.includes('人が差し戻した'));
  // Without the comment on the whole, and with one point.
  const [, , one] = events([rec({ answers: [{ decision: 'changes', answeredAt: '20261012T0005', handoff: { ...handoff, count: 1, overall: false } }] })]);
  assert.ok(one.includes('AI 下読みの指示 1 件を worker に渡した — ') && !one.includes('全体のコメントも') && !one.includes('<details'));
});

test('a waiting gate answered with them says so too, and an ordinary send-back is as it was', () => {
  const waiting = rec({ id: 'g1', wait: true, answeredAt: '20261012T0005', decision: 'changes', comment: 'the message', handoff });
  const [, , sent] = events([waiting]);
  assert.ok(sent.includes('AI 下読みの指示 3 件を worker に渡した・全体のコメントも — コードレビュー「Round the amounts」'), sent);
  const plain = events([rec({ answers: [{ decision: 'changes', comment: 'fix it', answeredAt: '20261012T0005' }] })])[2];
  assert.ok(plain.includes('人が差し戻した — コードレビューの記録「Round the amounts」') && plain.includes('<div class="who">fix it</div>') && !plain.includes('AI 下読み'));
});

test('the 経過 of a gate in the decision column lists the handed points under their time, and 離れていた間に names them in its last action', () => {
  const decide = read('decide.js');
  assert.match(decide, /a\.handoff\s*\?[^]*handoffText\(a\.handoff\)[^]*handoffDetailsHtml\(a\.comment\)/);
  const away = new vm.createContext({});
  for (const file of ['util.js', 'my-work-seen.js', 'my-work-away.js']) vm.runInContext(read(file), away, { filename: file });
  const { secsStamp } = vm.runInContext('({ secsStamp })', away);
  const T0 = Date.UTC(2026, 9, 12) / 1000;
  const e = { task: null, session: null };
  const last = (gates) => vm.runInContext('workLastAction', away)(e, gates);
  const got = last([{ kind: 'diff', title: 'T', answers: [{ answeredAt: secsStamp(T0), handoff }] }]);
  assert.strictEqual(got.text, 'AI 下読みの指示 3 件を worker に渡した・全体のコメントも — コードレビューの記録『T』');
  assert.strictEqual(last([{ kind: 'diff', title: 'T', answers: [{ answeredAt: secsStamp(T0) }] }]).text, 'コードレビューの記録『T』を差し戻した');
  const waited = last([{ kind: 'diff', title: 'T', decision: 'changes', answeredAt: secsStamp(T0), handoff: { ...handoff, overall: false } }]);
  assert.strictEqual(waited.text, 'AI 下読みの指示 3 件を worker に渡した — コードレビュー『T』');
});
