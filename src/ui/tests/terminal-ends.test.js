/* What a terminal that ended says (`terminalEndText` in src/ui/terminal.js), run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const block = src.match(/const TERMINAL_ENDS = \{[\s\S]*?\n\};\nfunction terminalEndText[\s\S]*?\n\}\n/);
assert.ok(block, 'terminalEndText not found in terminal.js');
const ctx = vm.createContext({});
vm.runInContext(`${block[0]}; this.text = terminalEndText;`, ctx);
const text = ctx.text;

test('a window that is gone and a terminal that could not start are said apart', () => {
  assert.strictEqual(text(4410), 'ウィンドウは終わっています');
  assert.strictEqual(text(0, 'xterm.js を読み込めませんでした'), 'xterm.js を読み込めませんでした');
  assert.strictEqual(text(0), '接続できませんでした');
});

test('tmux failing says why, and a code the page does not know is shown as it is', () => {
  assert.strictEqual(text(4500, 'boom'), 'tmux の操作に失敗しました: boom');
  assert.strictEqual(text(4999), '接続が閉じました（4999）');
});

test('a failed attach of a PR\'s reader says why: the code\'s words, then the server\'s reason unless it is said already', () => {
  const panel = fs.readFileSync(path.join(__dirname, '..', 'pr-panel.js'), 'utf8');
  const fn = panel.match(/^const prFailedText = [\s\S]*?\n\};/m)[0];
  const c = vm.createContext({ terminalEndText: text });
  vm.runInContext(`${fn}; this.failed = prFailedText;`, c);
  assert.strictEqual(c.failed(4404, 'no reader window for this pull request'), 'セッションが見つからないか、tmux 上にありません: no reader window for this pull request');
  assert.strictEqual(c.failed(4500, 'boom'), 'tmux の操作に失敗しました: boom');
  assert.strictEqual(c.failed(0, 'xterm.js を読み込めませんでした'), 'xterm.js を読み込めませんでした');
  assert.strictEqual(c.failed(1006, ''), '接続できませんでした');
});
