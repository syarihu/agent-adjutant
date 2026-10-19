/* The address of a PR waiting on your review (src/ui/nav.js, core.js), run with `node --test`: it stays on 「すべて」, in the
   work view, with its own panes, and round-trips. The page's scripts share one global scope, so what is needed is cut out of
   them and loaded into one context. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const navSrc = read('nav.js');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};

function world(multiBoard = true) {
  const c = vm.createContext({ URLSearchParams, TOKEN: 't', multiBoard, nav: { board: null, view: 'agent', task: null, pane: 'detail' }, location: {},
    HUB_REF: 'hub:', SESS_REF: 'session:', PARENT_REF: 'parent:', WORK_GATE_REF: 'gate:' });
  vm.runInContext([
    read('nav-legacy.js'),
    cut(read('core.js'), /^const PR_REF = [^\n]*;\nconst isPrRef = [^\n]*;\nconst prIdOfRef = [^\n]*;\nconst PR_PANES = [^\n]*;/m),
    cut(read('core.js'), /^const READ_PANES = [^\n]*;\n\/\* The ref of a task[^\n]*\nconst isTaskRef = [^\n]*;/m),
    cut(navSrc, /^const PANES = [^\n]*;/m),
    cut(navSrc, /^const DEFAULT_MULTI_VIEW = [^\n]*;/m),
    cut(navSrc, /^function parseUrl\([^)]*\) \{[\s\S]*?\n\}/m),
    cut(navSrc, /^function urlOf\([^)]*\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c);
  const parse = url => {
    const u = new URL(url, 'http://adj.test');
    return JSON.parse(JSON.stringify(c.parseUrl({ pathname: u.pathname, search: u.search, hash: u.hash })));
  };
  const urlOf = n => c.urlOf(n);
  return { parse, urlOf };
}

const PR = 'pr:acme/widget#12';
const enc = encodeURIComponent(PR);

test('a PR in the address keeps its task and its pane on 「すべて」, in the work view', () => {
  const w = world();
  assert.deepStrictEqual(w.parse(`/?token=t&view=work&task=${enc}`), { board: 'all', view: 'work', task: PR, pane: 'detail' });
  for (const pane of ['detail', 'diff', 'drafts']) {
    assert.deepStrictEqual(w.parse(`/?token=t&view=work&task=${enc}&pane=${pane}`), { board: 'all', view: 'work', task: PR, pane });
  }
  assert.strictEqual(w.parse(`/?token=t&view=work&task=${enc}&pane=nonsense`).pane, 'detail');
});

test('the PR panes belong to a PR: another task falls back to 詳細, and a PR is never kept outside the work view', () => {
  const w = world();
  assert.strictEqual(w.parse('/b/acme/?token=t&task=t1&pane=drafts').pane, 'detail');
  assert.strictEqual(w.parse('/b/acme/?token=t&task=t1&pane=review').pane, 'review');
  // A task on 「すべて」 is dropped as before; so is a PR when the view is not the work view.
  assert.strictEqual(w.parse('/?token=t&view=work&task=t1').task, null);
  assert.strictEqual(w.parse(`/?token=t&view=agent&task=${enc}`).task, null);
  assert.strictEqual(w.parse(`/?token=t&view=human&task=${enc}`).task, null);
  // A board served on its own has no work view.
  assert.strictEqual(world(false).parse(`/?token=t&view=work&task=${enc}`).task, null);
});

test('the address round-trips, and the plain link has no pane', () => {
  const w = world();
  const nav = { board: 'all', view: 'work', task: PR, pane: 'drafts' };
  assert.strictEqual(w.urlOf(nav), `/?token=t&view=work&task=${enc}&pane=drafts`);
  assert.deepStrictEqual(w.parse(w.urlOf(nav)), nav);
  const plainLink = w.urlOf({ ...nav, pane: 'detail' });
  assert.ok(!plainLink.includes('pane='));
  assert.deepStrictEqual(w.parse(plainLink), { ...nav, pane: 'detail' });
  // The `#` and `/` of the id are escaped, so the link survives being pasted.
  assert.ok(plainLink.includes('task=pr%3Aacme%2Fwidget%2312'));
});

test('a malformed escape in the address does not throw', () => {
  const w = world();
  assert.doesNotThrow(() => w.parse('/?token=t&view=work&task=pr%3Aacme%2Fwid%E0%A4%A&pane=drafts'));
});

test('a board\'s own address does not keep a PR: it never reads the records', () => {
  const w = world();
  assert.strictEqual(w.parse(`/b/acme/?token=t&view=work&task=${enc}`).task, null);
});

test('a task\'s read-through panes are kept for a task in the work view, and for nothing else', () => {
  const w = world();
  for (const pane of ['ai', 'ai-drafts']) {
    assert.deepStrictEqual(w.parse(`/b/acme/?token=t&view=work&task=t1&pane=${pane}`), { board: 'acme', view: 'work', task: 't1', pane });
    assert.strictEqual(w.urlOf({ board: 'acme', view: 'work', task: 't1', pane }), `/b/acme/?token=t&view=work&task=t1&pane=${pane}`);
  }
  // Outside the work view the task has the board's panes only.
  assert.strictEqual(w.parse('/b/acme/?token=t&task=t1&pane=ai').pane, 'detail');
  assert.strictEqual(w.parse('/b/acme/?token=t&view=human&task=t1&pane=ai-drafts').pane, 'detail');
  // Not a task: a hub, a session, a parent, a gate, a review request, or nothing.
  for (const ref of ['hub:h1', 'session:s1', 'parent:acme%2Fw%231', 'gate:acme%2Fg1']) {
    assert.strictEqual(w.parse(`/b/acme/?token=t&view=work&task=${ref}&pane=ai`).pane, 'detail', ref);
  }
  assert.strictEqual(w.parse(`/?token=t&view=work&task=${enc}&pane=ai`).pane, 'detail');
  assert.strictEqual(w.parse('/b/acme/?token=t&view=work&pane=ai').pane, 'detail');
  // On 「すべて」 the task is dropped, and so is its pane.
  assert.strictEqual(w.parse('/?token=t&view=work&task=t1&pane=ai').pane, 'detail');
});

test('`/` opens the work view, and 「すべて」 is asked for by name', () => {
  const w = world();
  assert.deepStrictEqual(w.parse('/?token=t'), { board: 'all', view: 'work', task: null, pane: 'detail' });
  assert.deepStrictEqual(w.parse('/?token=t&view=agent'), { board: 'all', view: 'agent', task: null, pane: 'detail' });
  assert.strictEqual(w.urlOf({ board: 'all', view: 'agent', task: null, pane: 'detail' }), '/?token=t&view=agent');
  assert.deepStrictEqual(w.parse(w.urlOf({ board: 'all', view: 'agent', task: null, pane: 'detail' })), { board: 'all', view: 'agent', task: null, pane: 'detail' });
  // A board's own address is the agent board, with no `view` in it; a board served alone is too.
  assert.deepStrictEqual(w.parse('/b/acme/?token=t'), { board: 'acme', view: 'agent', task: null, pane: 'detail' });
  assert.strictEqual(w.urlOf({ board: 'acme', view: 'agent', task: null, pane: 'detail' }), '/b/acme/?token=t');
  assert.deepStrictEqual(world(false).parse('/?token=t'), { board: null, view: 'agent', task: null, pane: 'detail' });
  assert.strictEqual(world(false).urlOf({ board: null, view: 'agent', task: null, pane: 'detail' }), '/?token=t');
});
