/* The current-organisation API (src/ui/org.js), run with `node --test`.
   org.js reads the page's prefs, boards, work and nav, so it is evaluated here against stubs of those. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'org.js'), 'utf8');
const plain = v => JSON.parse(JSON.stringify(v));

/* A fresh page state each time: org.js is run whole, as the page does. */
function page({ prefs = {}, boards = [], doc = null, board = null } = {}) {
  const events = [];
  const listeners = [];
  const saved = [];
  const ctx = vm.createContext({
    prefs, boards, nav: { board }, work: { doc }, saved, events,
    savePrefs: () => saved.push(plain(prefs)),
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    document: {
      addEventListener: (type, fn) => listeners.push([type, fn]),
      dispatchEvent: e => { events.push(plain(e.detail)); for (const [t, fn] of listeners) if (t === e.type) fn(e); },
    },
  });
  vm.runInContext(src + '\nthis.api = { orgOf, orgList, orgWaitCounts, orgIcon, currentOrg, setCurrentOrg, onOrgChange, inCurrentOrg, ORG_EVENT };', ctx);
  return { ctx, api: ctx.api, events, saved };
}
const board = (slug, nwo, finished = false) => ({ slug, nwo, finished });

test('orgOf is the lowercase owner, and empty for an nwo with none', () => {
  const { api } = page();
  assert.strictEqual(api.orgOf('Acme/Widget'), 'acme');
  assert.strictEqual(api.orgOf('widget'), '');
  assert.strictEqual(api.orgOf('/widget'), '');
  assert.strictEqual(api.orgOf(null), '');
});

test('orgList: an owner with one repository is its own organisation, by name, repositories sorted', () => {
  const { api } = page();
  const list = plain(api.orgList([board('a', 'zeta/one'), board('b', 'acme/widget'), board('c', 'acme/api')], null));
  assert.deepStrictEqual(list.map(o => o.login), ['acme', 'zeta']);
  assert.deepStrictEqual(list[0].repos, ['acme/api', 'acme/widget']);
  assert.deepStrictEqual(list[1].repos, ['zeta/one']);
});

test('orgList: the first spelling names the organisation and a repository is listed once', () => {
  const { api } = page();
  const list = plain(api.orgList([board('a', 'Acme/Widget'), board('b', 'acme/widget')], { repos: [{ nwo: 'ACME/widget' }] }));
  assert.strictEqual(list.length, 1);
  assert.strictEqual(list[0].name, 'Acme');
  assert.deepStrictEqual(list[0].repos, ['Acme/Widget']);
});

test('orgList: finished boards are ignored, repositories only the document lists are included', () => {
  const { api } = page();
  const list = plain(api.orgList([board('a', 'acme/old', true), board('b', 'acme/widget')], { repos: [{ nwo: 'beta/tool' }] }));
  assert.deepStrictEqual(list.map(o => [o.login, o.repos]), [['acme', ['acme/widget']], ['beta', ['beta/tool']]]);
});

test('orgList: a repository with no owner is the organisation without a login, last', () => {
  const { api } = page();
  const list = plain(api.orgList([board('a', 'local'), board('b', 'acme/widget')], null));
  assert.deepStrictEqual(list.map(o => o.login), ['acme', '']);
  assert.strictEqual(list[1].name, 'オーナーなし');
});

test('orgWaitCounts counts the TODO\'s gates in the organisation of their repository', () => {
  const { api } = page();
  const orgs = [{ login: 'acme' }, { login: 'beta' }, { login: '' }];
  const counts = plain([...api.orgWaitCounts({
    quick: [{ nwo: 'Acme/a' }], read: [{ nwo: 'acme/b' }, { nwo: 'beta/x' }, { nwo: 'local' }, { nwo: 'stranger/z' }], count: 5,
  }, orgs)]);
  // An organisation with no registered repository is not counted: the menu has no entry to go to.
  assert.deepStrictEqual(counts, [['acme', 2], ['beta', 1], ['', 1]]);
  assert.strictEqual(api.orgWaitCounts(null, orgs).size, 0);
});

test('orgIcon finds an owner of the document by login, whatever the case', () => {
  const { api } = page({ doc: { owners: [{ login: 'Acme', avatarUrl: 'https://a/x' }] } });
  assert.strictEqual(api.orgIcon('acme').avatarUrl, 'https://a/x');
  assert.strictEqual(api.orgIcon('beta'), null);
  assert.strictEqual(page().api.orgIcon('acme'), null);
});

test('currentOrg is null while no repository is known', () => {
  assert.strictEqual(page({ prefs: { org: 'acme' } }).api.currentOrg(), null);
});

test('currentOrg: the saved one, else the board in the address, else the first', () => {
  const boards = [board('a', 'acme/widget'), board('b', 'beta/tool')];
  assert.strictEqual(page({ prefs: { org: 'beta' }, boards, board: 'a' }).api.currentOrg(), 'beta');
  assert.strictEqual(page({ prefs: { org: 'gone' }, boards, board: 'b' }).api.currentOrg(), 'beta');
  assert.strictEqual(page({ prefs: { org: '' }, boards, board: 'b' }).api.currentOrg(), 'beta');
  assert.strictEqual(page({ prefs: { org: 'gone' }, boards, board: 'all' }).api.currentOrg(), 'acme');
});

test('currentOrg moves off a saved organisation that is unregistered, without writing the prefs', () => {
  const p = page({ prefs: { org: 'beta' }, boards: [board('a', 'acme/widget'), board('b', 'beta/tool')] });
  assert.strictEqual(p.api.currentOrg(), 'beta');
  p.ctx.boards = [board('a', 'acme/widget')];
  assert.strictEqual(p.api.currentOrg(), 'acme');
  assert.strictEqual(p.ctx.prefs.org, 'beta');
  assert.strictEqual(p.saved.length, 0);
});

test('setCurrentOrg saves and tells the listeners, once, and refuses an unknown organisation', () => {
  const p = page({ prefs: { org: 'acme' }, boards: [board('a', 'acme/widget'), board('b', 'beta/tool')] });
  const told = [];
  p.api.onOrgChange((org, prev) => told.push([org, prev]));
  p.api.setCurrentOrg('Beta');
  assert.deepStrictEqual(told, [['beta', 'acme']]);
  assert.strictEqual(p.ctx.prefs.org, 'beta');
  assert.strictEqual(p.saved.length, 1);
  p.api.setCurrentOrg('beta');
  p.api.setCurrentOrg('nobody');
  assert.strictEqual(told.length, 1);
  assert.strictEqual(p.saved.length, 1);
  assert.strictEqual(p.api.ORG_EVENT, 'adj:org');
});

test('picking the organisation already in effect by the address saves it but tells nobody', () => {
  const p = page({ prefs: { org: '' }, boards: [board('a', 'acme/widget'), board('b', 'beta/tool')], board: 'b' });
  p.api.setCurrentOrg('beta');
  assert.strictEqual(p.events.length, 0);
  assert.strictEqual(p.ctx.prefs.org, 'beta');
});

test('the owner-less organisation can be selected and is saved without an empty pref', () => {
  const p = page({ prefs: { org: 'acme' }, boards: [board('a', 'acme/widget'), board('b', 'local')] });
  p.api.setCurrentOrg('');
  assert.strictEqual(p.ctx.prefs.org, '-');
  assert.strictEqual(p.api.currentOrg(), '');
  assert.deepStrictEqual(p.events, [{ org: '', prev: 'acme' }]);
});

test('inCurrentOrg compares the owner of a repository with the selected organisation', () => {
  const { api } = page({ prefs: { org: 'acme' }, boards: [board('a', 'acme/widget'), board('b', 'beta/tool')] });
  assert.ok(api.inCurrentOrg('ACME/other'));
  assert.ok(!api.inCurrentOrg('beta/tool'));
  assert.ok(!api.inCurrentOrg(''));
});

test('the prefs are normalised on load: org and workRepo are lowercase strings', () => {
  const p = page({ prefs: { org: 'Acme', workRepo: 'Acme/Widget' } });
  assert.strictEqual(p.ctx.prefs.org, 'acme');
  assert.strictEqual(p.ctx.prefs.workRepo, 'acme/widget');
  const q = page({ prefs: { org: 5, workRepo: null } });
  assert.strictEqual(q.ctx.prefs.org, '');
  assert.strictEqual(q.ctx.prefs.workRepo, '');
});
