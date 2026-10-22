/* The banner's pure parts (scFailing, scKey, scVisible, scHtml, scAnswered in src/ui/server-checks.js), run with `node --test`. The page's
   scripts share one global scope, so the file is loaded into one context. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const ctx = vm.createContext({ document: { getElementById: () => null }, esc });
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'server-checks.js'), 'utf8') + '\nthis.api = { scFailing, scKey, scVisible, scHtml, scAnswered };', ctx, { filename: 'server-checks.js' });
const { scFailing, scKey, scVisible, scHtml, scAnswered } = ctx.api;

const plain = x => JSON.parse(JSON.stringify(x));
const check = (id, ok, extra = {}) => ({ id, ok, fatal: false, message: `${id} says`, fix: ok ? null : `fix ${id}`, ...extra });

test('only the failed checks are listed, and an answer that is not a list has none', () => {
  const data = { checks: [check('tmux', true), check('gh', false), check('mcp', false)] };
  assert.deepStrictEqual(plain(scFailing(data).map(c => c.id)), ['gh', 'mcp']);
  assert.deepStrictEqual(plain(scFailing(null)), []);
  assert.deepStrictEqual(plain(scFailing({ error: 'no such route' })), []);
});

test('the banner shows while something fails and goes once every check passes', () => {
  assert.strictEqual(scVisible([], null), false);
  assert.strictEqual(scVisible([check('gh', false)], null), true);
});

test('a closed banner stays closed for the same failures and comes back for a new one', () => {
  const gh = [check('gh', false)];
  const key = scKey(gh);
  assert.strictEqual(scVisible(gh, key), false);
  assert.strictEqual(scVisible([check('mcp', false), check('gh', false)], key), true);
  assert.strictEqual(scKey([check('mcp', false), check('gh', false)]), scKey([check('gh', false), check('mcp', false)]));
});

test('every string of the server is escaped, and a fatal check is styled as an error', () => {
  const html = scHtml([check('gh', false, { message: '<img src=x onerror=1>', fix: '"; rm', fatal: true })]);
  assert.ok(!html.includes('<img'), html);
  assert.ok(html.includes('&lt;img'), html);
  assert.ok(html.includes('&quot;; rm'), html);
  assert.ok(html.includes('sc-error'), html);
  assert.ok(scHtml([check('gh', false)]).includes('sc-warn'));
});

test('an id the page has no label for falls back to the server message', () => {
  assert.ok(scHtml([check('new-one', false)]).includes('new-one says'));
  assert.ok(scHtml([check('tmux', false)]).includes('tmux が見つかりません'));
});

test('an answer from before the first check is not taken as everything passing', () => {
  assert.strictEqual(scAnswered({ checkedAt: null, checks: [] }), false);
  assert.strictEqual(scAnswered(null), false);
  assert.strictEqual(scAnswered({ checkedAt: 1700000000, checks: [] }), true);
});
