/* The first-run guide's pure parts (egShows, egSteps, egHtml in src/ui/empty-guide.js), run with `node --test`. The page's
   scripts share one global scope, so server-checks.js and empty-guide.js are loaded into one context. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const ctx = vm.createContext({ document: { getElementById: () => null }, esc });
const load = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
vm.runInContext(load('server-checks.js') + '\n' + load('empty-guide.js') + '\nthis.api = { egShows, egSteps, egHtml };', ctx, { filename: 'empty-guide.js' });
const { egShows, egSteps, egHtml } = ctx.api;

const plain = x => JSON.parse(JSON.stringify(x));
const base = { multiBoard: true, boardsLoaded: true, boardCount: 0, board: 'all', view: 'agent', list: 'task', task: null };
const check = (id, ok, extra = {}) => ({ id, ok, fatal: false, message: `${id} says`, fix: ok ? null : `fix ${id}`, ...extra });
const answer = checks => ({ checkedAt: 1700000000, checks });

test('the guide shows on ボード and on the empty いまの仕事 once the list is read and empty', () => {
  assert.strictEqual(egShows(base), true);
  assert.strictEqual(egShows({ ...base, view: 'work' }), true);
});

test('the guide does not show before the list is read, on a single board, or with boards', () => {
  assert.strictEqual(egShows({ ...base, boardsLoaded: false }), false);
  assert.strictEqual(egShows({ ...base, multiBoard: false }), false);
  assert.strictEqual(egShows({ ...base, boardCount: 1 }), false);
  assert.strictEqual(egShows(null), false);
});

test('the guide leaves レビュー, TODO and an open task as they are', () => {
  assert.strictEqual(egShows({ ...base, view: 'work', list: 'review' }), false);
  assert.strictEqual(egShows({ ...base, view: 'todo' }), false);
  assert.strictEqual(egShows({ ...base, view: 'work', task: 't1' }), false);
  assert.strictEqual(egShows({ ...base, view: 'agent', task: 't1' }), false);
  assert.strictEqual(egShows({ ...base, view: 'agent', board: 'owner-repo' }), false);
});

test('with nothing failing, or no answer, only the adj hub step is listed', () => {
  for (const data of [null, answer([]), answer([check('tmux', true)]), { checkedAt: null, checks: [] }]) {
    const steps = plain(egSteps(data));
    assert.strictEqual(steps.length, 1);
    assert.strictEqual(steps[0].fix, 'adj hub');
  }
});

test('with a failure, every check is listed in the server order, passed ones done, and adj hub comes last', () => {
  const steps = plain(egSteps(answer([check('tmux', true), check('gh', false), check('mcp', false, { fatal: true })])));
  assert.deepStrictEqual(steps.map(s => s.id), ['tmux', 'gh', 'mcp', undefined]);
  assert.deepStrictEqual(steps.map(s => s.done), [true, false, false, undefined]);
  assert.strictEqual(steps[1].label, 'gh をインストールする');
  assert.strictEqual(steps[0].label, 'tmux があります');
  assert.strictEqual(steps[3].fix, 'adj hub');
  const html = egHtml(egSteps(answer([check('tmux', true), check('gh', false), check('mcp', false, { fatal: true })])));
  assert.ok(html.includes('check_circle') && html.includes('radio_button_unchecked'), html);
  assert.ok(html.includes('eg-done') && html.includes('eg-todo'), html);
  assert.ok(html.includes('eg-todo eg-error'), html);
  assert.ok(html.includes('data-eg-recheck'), html);
  assert.ok(html.includes('data-eg-copy="fix gh"') && html.includes('data-eg-copy="adj hub"'), html);
  assert.ok(html.includes('data-action="review"'), html);
});

test('the recheck button appears only with check steps', () => {
  assert.ok(!egHtml(egSteps(null)).includes('data-eg-recheck'));
});

test('every string of the server is escaped', () => {
  const html = egHtml(egSteps(answer([check('gh', false, { message: '<img src=x onerror=1>', fix: '"; <img rm' })])));
  assert.ok(!html.includes('<img'), html);
  assert.ok(html.includes('&lt;img src=x'), html);
  assert.ok(html.includes('&quot;; &lt;img rm'), html);
});

test('an id the page has no label for falls back to the server message, then the id', () => {
  const steps = plain(egSteps(answer([check('new-one', false), check('other', true, { message: '' })])));
  assert.strictEqual(steps[0].label, 'new-one says');
  assert.strictEqual(steps[1].label, 'other');
});
