/* The 「依頼する」 dialog (src/ui/request-dialog.js), run with `node --test`.
   The file binds to the page as it loads, so the pure helpers are cut out of the source and run here. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'request-dialog.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const plain = v => JSON.parse(JSON.stringify(v));
const ctx = vm.createContext({});
vm.runInContext([
  cut(/^function requestHubOptions[\s\S]*?^}/m),
  cut(/^const requestRepos = [^\n]*;/m),
  cut(/^function requestPickBoard[\s\S]*?^}/m),
  cut(/^const requestBody = [^\n]*;/m),
  cut(/^const requestSendKey = [^\n]*;/m),
].join('\n'), ctx);
const run = (code, vars = {}) => plain(Object.assign(ctx, vars) && vm.runInContext(code, ctx));
const b = (slug, nwo, extra = {}) => ({ slug, nwo, hub: null, finished: false, ...extra });
const boards = [
  b('w-p2', 'acme/widget', { hub: 'ALPHA-2', title: 'Second' }), b('w', 'Acme/Widget'), b('w-p1', 'acme/widget', { hub: 'ALPHA-1' }),
  b('w-old', 'acme/widget', { hub: 'OLD-1', finished: true }), b('t', 'acme/tool'), b('z', 'zeta/z'),
];

test('the hubs of a repository: its own first, then the parents by key, and a finished one is out', () => {
  assert.deepStrictEqual(run('requestHubOptions(boards, "acme/widget")', { boards }), [
    { slug: 'w', label: 'リポジトリ hub' }, { slug: 'w-p1', label: '親 hub · ALPHA-1' }, { slug: 'w-p2', label: '親 hub · ALPHA-2 Second' }]);
  assert.deepStrictEqual(run('requestHubOptions(boards, "nobody/none")', { boards }), []);
});

test('the repositories to ask are the organisation\'s that have a board to take the request', () => {
  const orgs = [{ login: 'acme', repos: ['acme/tool', 'acme/widget', 'acme/gone'] }, { login: 'zeta', repos: ['zeta/z'] }];
  assert.deepStrictEqual(run('requestRepos(orgs, boards, "acme")', { orgs, boards }), ['acme/tool', 'acme/widget']);
  assert.deepStrictEqual(run('requestRepos(orgs, boards, "none")', { orgs, boards }), []);
});

test('the board it opens on: the one named, else the address\'s, else the filter\'s repository, else the first', () => {
  const pick = o => run('requestPickBoard(boards, o)?.slug ?? null', { boards, o });
  assert.strictEqual(pick({ slug: 'w-p1', navBoard: 't' }), 'w-p1');
  assert.strictEqual(pick({ navBoard: 't' }), 't');
  // A finished board is not a place to ask; the work view keeps to the organisation shown.
  assert.strictEqual(pick({ navBoard: 'w-old', org: 'acme' }), 'w');
  assert.strictEqual(pick({ navBoard: 'z', inWork: true, org: 'acme' }), 'w');
  assert.strictEqual(pick({ navBoard: 'z', inWork: false, org: 'acme' }), 'z');
  assert.strictEqual(pick({ org: 'acme', workRepo: 'acme/tool' }), 't');
  assert.strictEqual(pick({ org: 'zeta' }), 'z');
  assert.strictEqual(run('requestPickBoard([], {})'), null);
});

test('the request is the person\'s words, needing reading, with the parent only when there is one', () => {
  assert.deepStrictEqual(run('requestBody("do it", "queued")'), { body: 'do it', status: 'queued', needsReading: true });
  assert.deepStrictEqual(run('requestBody("do it", "backlog", "https://x/1")'), { body: 'do it', status: 'backlog', needsReading: true, parent: 'https://x/1' });
});

test('Ctrl or ⌘ with Enter sends, a plain Enter does not, and an IME\'s Enter never does', () => {
  const send = e => run('requestSendKey(e)', { e });
  assert.strictEqual(send({ key: 'Enter', ctrlKey: true }), true);
  assert.strictEqual(send({ key: 'Enter', metaKey: true }), true);
  assert.strictEqual(send({ key: 'Enter' }), false);
  assert.strictEqual(send({ key: 'a', ctrlKey: true }), false);
  assert.strictEqual(send({ key: 'Enter', ctrlKey: true, isComposing: true }), false);
  assert.strictEqual(send({ key: 'Enter', metaKey: true, keyCode: 229 }), false);
});
