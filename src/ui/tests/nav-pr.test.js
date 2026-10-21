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
  const c = vm.createContext({ URLSearchParams, TOKEN: 't', multiBoard, nav: { board: null, view: 'agent', list: 'task', task: null, pane: 'detail' }, location: {},
    pendingOwnRead: null, HUB_REF: 'hub:', SESS_REF: 'session:', PARENT_REF: 'parent:', WORK_GATE_REF: 'gate:' });
  vm.runInContext([
    read('nav-legacy.js'),
    cut(read('core.js'), /^const PR_REF = [^\n]*;\nconst isPrRef = [^\n]*;\nconst prIdOfRef = [^\n]*;\nconst PR_PANES = [^\n]*;/m),
    cut(read('core.js'), /^\/\* The ref of a task[^\n]*\nconst isTaskRef = [^\n]*;/m),
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
  return { parse, urlOf, c };
}

const PR = 'pr:acme/widget#12';
const enc = encodeURIComponent(PR);

test('a PR in the address keeps its task and its pane on 「すべて」, in the work view', () => {
  const w = world();
  assert.deepStrictEqual(w.parse(`/?token=t&view=work&task=${enc}`), { board: 'all', view: 'work', list: 'review', task: PR, pane: 'detail' });
  for (const pane of ['detail', 'diff', 'drafts']) {
    assert.deepStrictEqual(w.parse(`/?token=t&view=work&task=${enc}&pane=${pane}`), { board: 'all', view: 'work', list: 'review', task: PR, pane });
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
  const nav = { board: 'all', view: 'work', list: 'review', task: PR, pane: 'drafts' };
  assert.strictEqual(w.urlOf(nav), `/?token=t&view=review&task=${enc}&pane=drafts`);
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

test('an older link to a task\'s AI 下読み opens the task on its own pane and asks for the dialog, and nothing else keeps the pane', () => {
  const w = world();
  for (const pane of ['ai', `ai-drafts`]) {
    w.c.pendingOwnRead = null;
    assert.deepStrictEqual(w.parse(`/b/acme/?token=t&view=work&task=t1&pane=${pane}`), { board: 'acme', view: 'work', list: 'task', task: 't1', pane: 'detail' });
    assert.strictEqual(w.c.pendingOwnRead, 'acme/t1', pane);
  }
  // The pane never comes back out in an address.
  assert.strictEqual(w.urlOf({ board: 'acme', view: 'work', list: 'task', task: 't1', pane: 'detail' }), '/b/acme/?token=t&view=work&task=t1');
  // Outside the work view, or on something that is not a task, the pane is dropped and no dialog is asked for.
  w.c.pendingOwnRead = null;
  assert.strictEqual(w.parse('/b/acme/?token=t&task=t1&pane=ai').pane, 'detail');
  assert.strictEqual(w.parse('/b/acme/?token=t&view=human&task=t1&pane=ai-drafts').pane, 'detail');
  for (const ref of ['hub:h1', 'session:s1', 'parent:acme%2Fw%231', 'gate:acme%2Fg1']) {
    assert.strictEqual(w.parse(`/b/acme/?token=t&view=work&task=${ref}&pane=ai`).pane, 'detail', ref);
  }
  // A PR keeps its own panes, and a stale one of a task's is the first of them.
  assert.deepStrictEqual(w.parse(`/?token=t&view=work&task=${enc}&pane=ai`), { board: 'all', view: 'work', list: 'review', task: PR, pane: 'detail' });
  assert.deepStrictEqual(w.parse(`/?token=t&view=review&task=${enc}&pane=ai-drafts`), { board: 'all', view: 'work', list: 'review', task: PR, pane: 'detail' });
  assert.strictEqual(w.parse('/b/acme/?token=t&view=work&pane=ai').pane, 'detail');
  // On 「すべて」 the task is dropped, and so is its dialog.
  assert.strictEqual(w.parse('/?token=t&view=work&task=t1&pane=ai').task, null);
  assert.strictEqual(w.c.pendingOwnRead, null);
});

test('`/` opens the work view, and 「すべて」 is asked for by name', () => {
  const w = world();
  assert.deepStrictEqual(w.parse('/?token=t'), { board: 'all', view: 'work', list: 'task', task: null, pane: 'detail' });
  assert.deepStrictEqual(w.parse('/?token=t&view=agent'), { board: 'all', view: 'agent', list: 'task', task: null, pane: 'detail' });
  assert.strictEqual(w.urlOf({ board: 'all', view: 'agent', list: 'task', task: null, pane: 'detail' }), '/?token=t&view=agent');
  assert.deepStrictEqual(w.parse(w.urlOf({ board: 'all', view: 'agent', list: 'task', task: null, pane: 'detail' })), { board: 'all', view: 'agent', list: 'task', task: null, pane: 'detail' });
  // A board's own address is the agent board, with no `view` in it; a board served alone is too.
  assert.deepStrictEqual(w.parse('/b/acme/?token=t'), { board: 'acme', view: 'agent', list: 'task', task: null, pane: 'detail' });
  assert.strictEqual(w.urlOf({ board: 'acme', view: 'agent', list: 'task', task: null, pane: 'detail' }), '/b/acme/?token=t');
  assert.deepStrictEqual(world(false).parse('/?token=t'), { board: null, view: 'agent', list: 'task', task: null, pane: 'detail' });
  assert.strictEqual(world(false).urlOf({ board: null, view: 'agent', list: 'task', task: null, pane: 'detail' }), '/?token=t');
});

test('レビュー has an address of its own, and an older PR link from the work view lands on it', () => {
  const w = world();
  assert.deepStrictEqual(w.parse('/?token=t&view=review'), { board: 'all', view: 'work', list: 'review', task: null, pane: 'detail' });
  assert.strictEqual(w.urlOf({ board: 'all', view: 'work', list: 'review', task: null, pane: 'detail' }), '/?token=t&view=review');
  assert.strictEqual(w.urlOf({ board: 'all', view: 'work', list: 'task', task: null, pane: 'detail' }), '/?token=t&view=work');
  // It survives a reload: what is written reads back as the same.
  const review = { board: 'all', view: 'work', list: 'review', task: PR, pane: 'diff' };
  assert.deepStrictEqual(w.parse(w.urlOf(review)), review);
  assert.deepStrictEqual(w.parse(`/?token=t&view=review&task=${enc}`), { board: 'all', view: 'work', list: 'review', task: PR, pane: 'detail' });
  assert.strictEqual(w.parse(`/?token=t&view=work&task=${enc}`).list, 'review');
  // いまの仕事 is the other list, with or without a task.
  assert.strictEqual(w.parse('/?token=t&view=work').list, 'task');
  assert.strictEqual(w.parse('/b/acme/?token=t&view=work&task=t1').list, 'task');
});

test('in レビュー a task that is not a PR is dropped, and a board served alone has no such view', () => {
  const w = world();
  for (const ref of ['t1', 'hub:h1', 'session:s1']) {
    assert.deepStrictEqual(w.parse(`/?token=t&view=review&task=${ref}`), { board: 'all', view: 'work', list: 'review', task: null, pane: 'detail' }, ref);
  }
  // A PR on a board's own address is dropped, and the list is the tasks'.
  assert.deepStrictEqual(w.parse(`/b/acme/?token=t&view=work&task=${enc}`).list, 'task');
  assert.strictEqual(world(false).parse('/?token=t&view=review').view, 'agent');
  assert.strictEqual(world(false).parse('/?token=t&view=review').list, 'task');
});

test('going back leaves no dialog armed, unless the address now shown asks for it', () => {
  const w = world();
  let handler;
  const c = vm.createContext({
    window: { addEventListener: (t, f) => { handler = f; } }, location: { pathname: '/b/acme/', search: '?token=t&view=work&task=t1' },
    pendingGate: 'g', pendingOwnRead: 'acme/t1', boardJob: {}, nav: { board: 'acme', view: 'agent', list: 'task', task: null, pane: 'detail' },
    parseUrl: w.c.parseUrl, sameNav: () => true,
  });
  vm.runInContext(cut(navSrc, /^window\.addEventListener\('popstate', \(\) => \{[\s\S]*?\n\}\);/m), c);
  handler();
  assert.strictEqual(c.pendingOwnRead, null);
  // The address shown carries the old pane: parseUrl arms it again (its context is the one that holds the variable).
  w.c.pendingOwnRead = null;
  w.parse('/b/acme/?token=t&view=work&task=t1&pane=ai');
  assert.strictEqual(w.c.pendingOwnRead, 'acme/t1');
});
