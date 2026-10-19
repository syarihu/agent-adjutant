/* The toasts' list (toastText, toastPush, toastLive in src/ui/toast.js), run with `node --test`. The page's scripts share
   one global scope, so the file is loaded into one context. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({ document: { getElementById: () => null, addEventListener() {} }, esc: s => s });
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'toast.js'), 'utf8') + '\nthis.api = { toastText, toastPush, toastLive };', ctx, { filename: 'toast.js' });
const { toastText, toastPush, toastLive } = ctx.api;
const plain = x => JSON.parse(JSON.stringify(x));

test('a reason leads and the line follows it; without one the line is the text', () => {
  assert.deepStrictEqual(plain(toastText('adj task done t1', false)), { text: 'adj task done t1', sub: '', err: false });
  assert.deepStrictEqual(plain(toastText('adj task done t1', true, '失敗しました')), { text: '失敗しました', sub: 'adj task done t1', err: true });
});

test('the newest toast is on top and no more than three are kept', () => {
  let list = [];
  for (const n of [1, 2, 3, 4]) list = toastPush(list, toastText(`line ${n}`, false), n);
  assert.deepStrictEqual(plain(list.map(t => t.text)), ['line 4', 'line 3', 'line 2']);
});

test('the same toast again replaces the one showing and restarts its time', () => {
  let list = toastPush([], toastText('poll failed', true), 1);
  list = toastPush(list, toastText('other', false), 2);
  list = toastPush(list, toastText('poll failed', true), 3);
  assert.deepStrictEqual(plain(list), [{ text: 'poll failed', sub: '', err: true, at: 3 }, { text: 'other', sub: '', err: false, at: 2 }]);
  // The same words as a note and as an error are two toasts.
  assert.strictEqual(toastPush(list, toastText('poll failed', false), 4).length, 3);
});

test('an error stays longer than a note', () => {
  const list = [{ text: 'a', sub: '', err: false, at: 0 }, { text: 'b', sub: '', err: true, at: 0 }];
  assert.deepStrictEqual(plain(toastLive(list, 3999).map(t => t.text)), ['a', 'b']);
  assert.deepStrictEqual(plain(toastLive(list, 4000).map(t => t.text)), ['b']);
  assert.deepStrictEqual(plain(toastLive(list, 10000).map(t => t.text)), []);
});
