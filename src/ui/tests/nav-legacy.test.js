/* Where old addresses land (src/ui/nav-legacy.js), run with `node --test`. The page's scripts share one global
   scope, so the file is loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({ URLSearchParams });
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'nav-legacy.js'), 'utf8'), ctx, { filename: 'nav-legacy.js' });
// Objects made inside the context have another Object.prototype: compare them as data.
const plain = x => JSON.parse(JSON.stringify(x));
const nav = (url, multi) => {
  const u = new URL(url, 'http://adj.test');
  return plain(ctx.legacyNav({ pathname: u.pathname, search: u.search, hash: u.hash }, multi));
};

const LIST = { board: 'all', view: 'work', task: null, pane: 'detail' };

test('addresses that are not old ones are left alone', () => {
  for (const url of ['/', '/?token=t', '/?view=work', '/b/acme/?view=human', '/b/acme/?task=hub:hub&pane=term',
    '/b/acme/?view=work&task=session:w1', '/b/acme/#task/t1/review', '/review/other']) {
    assert.strictEqual(nav(url, true), null, url);
    assert.strictEqual(nav(url, false), null, url);
  }
});

test('on the resident server the queue is the list of work, and a gate in it is opened once the document is in', () => {
  assert.deepStrictEqual(nav('/review?token=t', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/review?token=t&item=acme%2Fg1', true), { nav: LIST, pendingGate: 'acme/g1' });
  assert.deepStrictEqual(nav('/review?item=acme/g1', true), { nav: LIST, pendingGate: 'acme/g1' });
  // `#gate/<id>` on a board's own page names that board's gate.
  assert.deepStrictEqual(nav('/b/acme/?token=t#gate/g2', true), { nav: LIST, pendingGate: 'acme/g2' });
  assert.deepStrictEqual(nav('/b/acme/#gate/g%32', true), { nav: LIST, pendingGate: 'acme/g2' });
});

test('a permission wait or a bare id has no gate to open', () => {
  assert.deepStrictEqual(nav('/review?item=acme%2Fwait%3Aagent-1%3A1700000000', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/review?item=wait:agent-1:1700000000', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/review?item=g1', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/review?item=', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/#gate/g1', true), { nav: LIST, pendingGate: null });
});

test('the セッション tab of a board opens 「いまの仕事」 on that board, on the same task', () => {
  assert.deepStrictEqual(nav('/b/acme/?token=t&view=sessions&task=session%3Aw1&pane=term', true),
    { nav: { board: 'acme', view: 'work', task: 'session:w1', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/?view=sessions&task=hub%3Ahub&pane=term', true),
    { nav: { board: 'acme', view: 'work', task: 'hub:hub', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/?view=sessions&task=T-1', true),
    { nav: { board: 'acme', view: 'work', task: 'T-1', pane: 'detail' }, pendingGate: null });
  // The tabs of a task that are still tabs stay.
  for (const pane of ['review', 'check', 'history']) {
    assert.deepStrictEqual(nav(`/b/acme/?view=sessions&task=T-1&pane=${pane}`, true),
      { nav: { board: 'acme', view: 'work', task: 'T-1', pane }, pendingGate: null });
  }
  // A pane that is not one is the summary.
  assert.deepStrictEqual(nav('/b/acme/?view=sessions&task=T-1&pane=nonsense', true),
    { nav: { board: 'acme', view: 'work', task: 'T-1', pane: 'detail' }, pendingGate: null });
});

test('older spellings of a session open it too', () => {
  assert.deepStrictEqual(nav('/b/acme/?view=sessions&session=w1', true),
    { nav: { board: 'acme', view: 'work', task: 'session:w1', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/#session/w1', true),
    { nav: { board: 'acme', view: 'work', task: 'session:w1', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/#session/w%201', true),
    { nav: { board: 'acme', view: 'work', task: 'session:w 1', pane: 'detail' }, pendingGate: null });
  // A task named by the address wins over the older session=.
  assert.deepStrictEqual(nav('/b/acme/?view=sessions&task=T-1&session=w1', true),
    { nav: { board: 'acme', view: 'work', task: 'T-1', pane: 'detail' }, pendingGate: null });
});

test('the セッション tab with nothing named, or of every board, is the list', () => {
  assert.deepStrictEqual(nav('/b/acme/?view=sessions', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/#sessions', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/?view=sessions', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/?view=sessions&task=session:w1', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/#session/w1', true), { nav: LIST, pendingGate: null });
});

test('a board served alone has no list: the queue is its board, and a gate opens in its panel', () => {
  assert.deepStrictEqual(nav('/review?token=t', false), { nav: { view: 'agent' }, pendingGate: null });
  assert.deepStrictEqual(nav('/review?item=g1', false), { nav: { view: 'agent', task: 'gate:g1', pane: 'detail' }, pendingGate: null });
  // A link made on the resident server names the board too; the id is what this board knows.
  assert.deepStrictEqual(nav('/review?item=acme%2Fg1', false), { nav: { view: 'agent', task: 'gate:g1', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/?token=t#gate/g1', false), { nav: { view: 'agent', task: 'gate:g1', pane: 'detail' }, pendingGate: null });
  assert.deepStrictEqual(nav('/review?item=wait:agent-1:1700000000', false), { nav: { view: 'agent' }, pendingGate: null });
  assert.deepStrictEqual(nav('/?view=sessions', false), { nav: { view: 'agent' }, pendingGate: null });
  assert.deepStrictEqual(nav('/?view=sessions&task=session:w1&pane=term', false), { nav: { view: 'agent' }, pendingGate: null });
  assert.deepStrictEqual(nav('/#session/w1', false), { nav: { view: 'agent' }, pendingGate: null });
});

test('a malformed escape in a hand-edited link does not throw', () => {
  const urls = ['/review?item=%E0%A4%A', '/review?item=%', '/b/acme/#gate/%E0%A4%A', '/b/acme/#gate/%', '/#gate/%zz',
    '/b/acme/#session/%', '/b/acme/#session/%E0%A4%A', '/b/acme/?view=sessions&task=%&session=%E0',
    '/b/acme/?view=sessions&task=%E0%A4%A&pane=%'];
  for (const url of urls) {
    for (const multi of [true, false]) {
      // `new URL` keeps a malformed escape as it is, as the address bar does.
      assert.doesNotThrow(() => nav(url, multi), `${url} ${multi}`);
    }
  }
  // The gate whose id cannot be read opens nothing; the rest of the address still lands on the list.
  assert.deepStrictEqual(nav('/b/acme/#gate/%E0%A4%A', true), { nav: LIST, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/#gate/%E0%A4%A', false), { nav: { view: 'agent' }, pendingGate: null });
  assert.deepStrictEqual(nav('/b/acme/#session/%', true), { nav: LIST, pendingGate: null });
});
