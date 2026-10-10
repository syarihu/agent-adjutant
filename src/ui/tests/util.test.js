/* The Markdown the board draws (md) and the plain text it reduces to for one-line places (mdPlain), in
   src/ui/util.js, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'util.js'), 'utf8'), ctx, { filename: 'util.js' });
const { md, mdPlain } = ctx;

test('md escapes markup in text, code and tables', () => {
  for (const src of ['<script>alert(1)</script>', '`<img src=x onerror=alert(1)>`', '```\n<script>x</script>\n```',
    '| a |\n|---|\n| <img onerror=x> |']) {
    const html = md(src);
    assert.ok(!/<script|<img/.test(html), html);
    assert.ok(html.includes('&lt;'), html);
  }
});

test('md draws the markers it reads', () => {
  assert.ok(md('**x**').includes('<b>x</b>'));
  assert.ok(md('`c`').includes('<code>c</code>'));
  assert.ok(md('- a').includes('<ul><li>a</li></ul>'));
  assert.ok(md('# h').includes('<h3>h</h3>'));
  const t = md('| a | b |\n|---|---|\n| 1 | 2 |');
  assert.ok(t.includes('<table>') && t.includes('<th>a</th>') && t.includes('<td>2</td>'), t);
});

test('mdPlain drops inline and line markers', () => {
  assert.strictEqual(mdPlain('## Title'), 'Title');
  assert.strictEqual(mdPlain('a **b** *c* ~~d~~ `e`'), 'a b c d e');
  assert.strictEqual(mdPlain('- a\n* b\n  + c'), 'a\nb\n  c');
  assert.strictEqual(mdPlain('> quoted'), 'quoted');
  assert.strictEqual(mdPlain('see [the docs](https://example.com/x)'), 'see the docs');
  assert.strictEqual(mdPlain('**Fix `foo` now**'), 'Fix foo now');
  assert.strictEqual(mdPlain('**`x`** done'), 'x done');
  assert.strictEqual(mdPlain('[`a`](https://example.com/x)'), 'a');
});

test('mdPlain blanks what only frames something and keeps what is inside', () => {
  assert.strictEqual(mdPlain('```js\nconst **a** = 1;\n```'), '\nconst **a** = 1;\n');
  assert.strictEqual(mdPlain('---\n***\n___'), '\n\n');
  assert.strictEqual(mdPlain('| a | b |\n|:--|--:|\n| 1 | 2 |'), 'a · b\n\n1 · 2');
});

test('mdPlain leaves what md does not read', () => {
  assert.strictEqual(mdPlain('snake_case and __init__'), 'snake_case and __init__');
  assert.strictEqual(mdPlain('1. first'), '1. first');
  assert.strictEqual(mdPlain(undefined), '');
  assert.strictEqual(mdPlain('`**kept**`'), '**kept**');
});
