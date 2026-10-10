/* The agent's last message in the task panel (lastMessageHtml in src/ui/sessions-side.js), run with `node --test`. */
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

const ctx = vm.createContext({ state: { now: 1000 }, minutesSince: (a, b) => (b - a) / 60, agoLabel: m => `${m}分前` });
vm.runInContext([
  read('util.js'),
  cut(read('card-dialog.js'), /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  cut(read('sessions-side.js'), /^function lastMessageHtml\(.*\) \{[\s\S]*?\n\}/m),
].join('\n'), ctx);
const { lastMessageHtml } = ctx;

test('the last message is drawn as Markdown in a .body', () => {
  const html = lastMessageHtml({ lastMessage: '**x**' });
  assert.ok(html.includes('最後のメッセージ'));
  assert.ok(html.includes('<div class="body">'), html);
  assert.ok(html.includes('<b>x</b>'), html);
});

test('the last message is never HTML', () => {
  const html = lastMessageHtml({ lastMessage: '<script>alert(1)</script>' });
  assert.ok(!html.includes('<script'), html);
  assert.ok(html.includes('&lt;script'), html);
});

test('no message draws nothing', () => {
  assert.strictEqual(lastMessageHtml({ lastMessage: '' }), '');
  assert.strictEqual(lastMessageHtml({}), '');
});
