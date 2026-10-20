/* The TODO (and the rail's レビュー count) narrowed to the selected organisation (todo-view.js `todoInScope`), and the organisation switch's counts agreeing with
   what the TODO lists. The page's scripts share one global scope, so the ones needed are loaded into one context, as todo.test.js does. */
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
const plain = v => JSON.parse(JSON.stringify(v));

const prefs = { org: '' };
const page = { boards: [], nav: { board: null }, work: { doc: null } };
const ctx = vm.createContext({
  TextEncoder, TextDecoder, prefs, savePrefs() {}, document: { addEventListener() {}, dispatchEvent() {} }, CustomEvent: class {},
  get boards() { return page.boards; }, get nav() { return page.nav; }, get work() { return page.work; },
});
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(read('question-card.js'), 'question-card.js');
// my-work-clean.js is loaded whole (it only defines); the two page helpers it asks of my-work.js and task-view.js are stood in for.
run('const workMerged = r => (r.task ? r.task.prState === "merged" : r.s?.branchPr?.state === "merged"); const prRefNumber = pr => /(\\d+)$/.exec(pr || "")?.[1] || "";', 'page.js');
run(read('my-work-clean.js'), 'my-work-clean.js');
run(read('todo.js'), 'todo.js');
run(read('org.js'), 'org.js');
run(cut(read('todo-view.js'), /^const todoInScope = [^\n]*;/m), 'todo-view.js');

const stamp = secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
const T0 = Date.UTC(2026, 9, 1) / 1000;
const gate = (id, kind = 'plan') => ({ id, kind, title: `${kind} ${id}`, openedAt: stamp(T0), slug: 'b1', quick: false });
const turn = (taskId, gates) => ({ board: 'b1', task: { id: taskId, title: `Task ${taskId}` }, gates });
const repo = (nwo, turns) => ({ nwo, rows: [], hubSessions: [], turns });
const rec = (repoName, n) => ({ id: `${repoName}#${n}`, repo: repoName, number: n, title: `PR ${n}`, state: 'requested', author: 'octo', requestedAt: stamp(T0 + n), url: `https://github.com/${repoName}/pull/${n}` });
const doc = { now: T0, repos: [
  repo('acme/widget', [turn('t1', [gate('g1')]), turn('t2', [gate('g2', 'diff')])]),
  repo('beta/tool', [turn('t3', [gate('g3')])]),
] };
const records = [rec('acme/widget', 1), rec('beta/tool', 2), rec('beta/tool', 3), rec('stranger/repo', 4)];
const boards = [{ slug: 'w', nwo: 'acme/widget', finished: false }, { slug: 't', nwo: 'beta/tool', finished: false }];
const g = name => vm.runInContext(name, ctx);
const inScope = nwo => g('todoInScope')()(nwo);
const mine = scope => plain(g('todoItems')({ doc, inScope: scope }));

test('with an organisation selected the TODO lists only its gates', () => {
  Object.assign(page, { boards, work: { doc } });
  prefs.org = 'beta';
  const items = mine(inScope);
  assert.deepStrictEqual(items.read.map(w => w.nwo), ['beta/tool']);
  assert.strictEqual(items.count, 1);
  prefs.org = 'acme';
  const acme = mine(inScope);
  assert.deepStrictEqual(acme.read.map(w => w.nwo), ['acme/widget', 'acme/widget']);
  assert.strictEqual(acme.count, 2);
});

test('while no organisation is known the TODO is not narrowed', () => {
  Object.assign(page, { boards: [], work: { doc: null } });
  assert.strictEqual(inScope('anyone/anything'), true);
});

test('the switch\'s count per organisation is what the TODO lists once that organisation is selected', () => {
  Object.assign(page, { boards, work: { doc } });
  const orgs = g('orgList')(boards, doc);
  const counts = plain([...g('orgWaitCounts')(mine(() => true), orgs)]);
  assert.deepStrictEqual(counts.sort(), [['acme', 2], ['beta', 1]]);
  // What the TODO lists under a selection is that organisation's gates.
  for (const [login, n] of counts) {
    prefs.org = login;
    assert.strictEqual(mine(inScope).count, n, login);
  }
});

test('a review request of an organisation with no registered repository is counted under every selection', () => {
  Object.assign(page, { boards, work: { doc } });
  const waiting = scope => plain(g('othersWaiting')(records, scope)).map(r => r.id);
  assert.ok(!g('orgList')(boards, doc).some(o => o.login === 'stranger'));
  for (const login of ['acme', 'beta']) {
    prefs.org = login;
    assert.ok(waiting(inScope).includes('stranger/repo#4'), login);
  }
  prefs.org = 'beta';
  assert.deepStrictEqual(waiting(inScope), ['beta/tool#2', 'beta/tool#3', 'stranger/repo#4']);
  prefs.org = 'acme';
  assert.deepStrictEqual(waiting(inScope), ['acme/widget#1', 'stranger/repo#4']);
});
