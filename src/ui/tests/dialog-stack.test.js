/* The stack of dialogs (src/ui/dialog-stack.js): its core, which holds no page, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'util.js'), 'utf8'), ctx, { filename: 'util.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'dialog-stack.js'), 'utf8'), ctx, { filename: 'dialog-stack.js' });
const { dialogStackCore, dialogHeadHtml } = vm.runInContext('({ dialogStackCore, dialogHeadHtml })', ctx);

test('push, push, pop leaves the first layer on top', () => {
  const s = dialogStackCore();
  s.push({ key: 'card' });
  s.push({ key: 'diff' });
  assert.equal(s.depth(), 2);
  assert.equal(s.top().key, 'diff');
  assert.equal(s.pop().key, 'diff');
  assert.equal(s.depth(), 1);
  assert.equal(s.top().key, 'card');
});

test('closeAll hands back every layer once, the top first, and leaves nothing', () => {
  const s = dialogStackCore();
  for (const key of ['a', 'b', 'c']) s.push({ key });
  assert.deepEqual(s.closeAll().map(l => l.key), ['c', 'b', 'a']);
  assert.equal(s.depth(), 0);
  assert.equal(s.top(), null);
  assert.deepEqual(s.closeAll(), []);
});

test('popping the last layer empties the stack, and popping an empty one gives nothing', () => {
  const s = dialogStackCore();
  s.push({ key: 'only' });
  assert.equal(s.pop().key, 'only');
  assert.equal(s.depth(), 0);
  assert.equal(s.pop(), null);
});

test('a way back is wanted only from the second layer on', () => {
  const s = dialogStackCore();
  assert.equal(s.backWanted(), false);
  s.push({ key: 'a' });
  assert.equal(s.backWanted(), false);
  s.push({ key: 'b' });
  assert.equal(s.backWanted(), true);
  s.pop();
  assert.equal(s.backWanted(), false);
});

test('the head of a layer escapes its title and leaves the sub line out when there is none', () => {
  assert.equal(dialogHeadHtml({ title: '<b>x</b>' }), '<h2 class="qd-title">&lt;b&gt;x&lt;/b&gt;</h2>');
  assert.ok(dialogHeadHtml({ title: 't', sub: '<i>s</i>' }).startsWith('<div class="qd-sub"><i>s</i></div>'));
});
