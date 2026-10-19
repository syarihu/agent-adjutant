/* A gate's words and evidence (src/ui/gate-evidence.js) and the waits of the work document (`workWaits` in my-work-seen.js), run
   with `node --test`. Loaded into one context the way the browser shares its global scope. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const run = code => vm.runInContext(code, ctx);

const gate = (key, since, extra = {}) => ({ kind: 'gate', gate: 'plan', title: `G ${key}`, since, key, ...extra });
const entry = (id, gates, extra = {}) => ({ id, nwo: 'acme/widget', ref: id, task: null, gates, items: [...gates], ...extra });

test('waits are the open gates, oldest first, and a parked task has none', () => {
  const entries = [
    entry('t1', [gate('s/g2', 200), gate('s/g1', 100, { gate: 'question' })], { task: { id: '41', title: 'Timer' } }),
    entry('t2', [gate('s/g3', 50)], { items: [gate('s/g3', 50), { kind: 'parked', reason: 'later', since: 60 }] }),
    entry('t3', [gate('s/g4', 150)], { gateTitle: 'Own gate' }),
  ];
  const waits = run('workWaits')(entries);
  assert.deepEqual(waits.map(w => w.key), ['s/g1', 's/g4', 's/g2']);
  assert.equal(waits[0].title, 'Timer');
  assert.equal(waits[0].taskId, '41');
  assert.equal(waits[1].title, 'Own gate');
  assert.deepEqual(run('workWaits')([]), []);
  assert.deepEqual(run('workWaits')(null), []);
});

test('a wait carries the board, parent, ids and entry its gate is on', () => {
  const [w] = run('workWaits')([entry('s/41', [gate('acme-1/20260922T041233Z-plan', 100)], { task: { id: '41', title: 'Timer', parent: 'p40' } })]);
  assert.equal(w.board, 'acme-1');
  assert.equal(w.gateId, '20260922T041233Z-plan');
  assert.equal(w.id, 'acme-1/20260922T041233Z-plan');
  assert.equal(w.entryId, 's/41');
  assert.equal(w.parent, 'p40');
  assert.equal(run('workWaits')([entry('x', [gate('b/g', 1)])])[0].parent, null);
});

test('the first lines of a text are one plain line, cut at a length, and a focus line is the first line only', () => {
  assert.equal(run('gatePlainLine')('**one**\n\ntwo\nthree'), 'one two');
  assert.equal(run('gateFocusLine')('first\nsecond'), 'first');
  assert.equal(run('gatePlainLine')('x'.repeat(200)).length, 141);
  assert.equal(run('gatePlainLine')(''), '');
});

const DIFF = [
  'diff --git a/src/a.rs b/src/a.rs', '--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,2 +1,2 @@', ' keep', '-old <b>', '+new <b>',
  'diff --git a/src/b.rs b/src/b.rs', '--- a/src/b.rs', '+++ b/src/b.rs', '@@ -5,2 +5,2 @@', ' ctx b', '-gone', '+here',
].join('\n');
const evidence = (...a) => run('gateEvidenceParts')(...a).html;

test('the evidence is the hunk of the file a finding names, else the first hunk, capped, and escaped', () => {
  const named = evidence(DIFF, [{ location: 'src/b.rs:6', text: 't' }]);
  assert.ok(named.includes('src/b.rs') && named.includes('here'));
  assert.ok(!named.includes('src/a.rs'));
  const fallback = evidence(DIFF, [{ location: 'elsewhere.rs:1' }]);
  assert.ok(fallback.includes('src/a.rs') && !fallback.includes('src/b.rs'));
  assert.equal(evidence(DIFF, undefined), fallback);
  assert.ok(fallback.includes('old &lt;b&gt;') && !fallback.includes('<b>'));
  const lines = ['diff --git a/big b/big', '--- a/big', '+++ b/big', '@@ -1,30 +1,30 @@', ...Array.from({ length: 30 }, (_, i) => ` line ${i}`)].join('\n');
  assert.equal(evidence(lines, []).split('class="prd-l ').length - 1, 12);
  assert.equal(evidence(lines, [], null, 5).split('class="prd-l ').length - 1, 5);
  assert.equal(evidence('', []), '');
  assert.equal(evidence(null, null), '');
  assert.ok(!fallback.includes('data-prd-plus'), 'no ＋ button of the diff view');
});

const READ = { sha: 'abcdef0123456789', annotations: [{ path: 'src/b.rs', kind: 'ex', anchor: '+here', text: 'Explains <here>' },
  { path: 'src/b.rs', kind: 'ex', anchor: 'nowhere at all', text: 'Lost one' }, { path: 'src/zzz.rs', kind: 'ex', anchor: 'x', text: 'Orphan' }] };

test('the evidence has the reader annotations under their lines when the PR has a read-through, and drops what has no line', () => {
  const plainHtml = run('gateEvidenceParts')(DIFF, [{ location: 'src/b.rs:6' }]);
  assert.equal(plainHtml.annotated, false);
  const got = run('gateEvidenceParts')(DIFF, [{ location: 'src/b.rs:6' }], READ);
  assert.equal(got.annotated, true);
  assert.ok(got.html.includes('Explains &lt;here&gt;') && !got.html.includes('Explains <here>'));
  assert.ok(!got.html.includes('Lost one') && !got.html.includes('Orphan'));
  assert.ok(!got.html.includes('data-prd-todraft') && !got.html.includes('data-prd-ask') && !got.html.includes('prd-plus'));
  assert.ok(got.html.indexOf('here') < got.html.indexOf('Explains'));
});

test('two gate diffs shown together are each parsed once, however the redraws alternate between them', () => {
  const parse = run('gateParsedDiff');
  const other = DIFF.replace(/src\/a\.rs/g, 'src/z.rs');
  const a = parse(DIFF);
  const b = parse(other);
  assert.notStrictEqual(a, b);
  for (let i = 0; i < 3; i++) {
    assert.strictEqual(parse(DIFF), a);
    assert.strictEqual(parse(other), b);
  }
  // A bounded few: more than that lets the oldest go.
  for (let i = 0; i < 6; i++) parse(`${DIFF}\n# ${i}`);
  assert.notStrictEqual(parse(DIFF), a);
});
