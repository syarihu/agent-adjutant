/* The pure half of the aide's pane (src/ui/aide-display.js), run with `node --test`. Loaded into one context the way the
   browser shares its global scope, as my-work-away.test.js does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'aide-display.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
// Functions are on the context's global object, `const`s of a script are only in its scope.
const { aideEmptyDisplay, aideDisplayOf, aideDisplayWithWaits, registerAidePart, aideDisplayHtml, aidePartHtml, aideWaits, aidePinSub, aidePeekCount,
  aideAskRowText, aideStateOf, aideNewLineHtml, aideHeadHtml, aideScopeText, aideCleanText, kindOf, aideRequestSteps, aideGateTagText } = vm.runInContext(
  `({ aideEmptyDisplay, aideDisplayOf, aideDisplayWithWaits, registerAidePart, aideDisplayHtml, aidePartHtml, aideWaits, aidePinSub, aidePeekCount,
     aideAskRowText, aideStateOf, aideNewLineHtml, aideHeadHtml, aideScopeText, aideCleanText, kindOf, aideRequestSteps, aideGateTagText })`, ctx);

const count = (html, needle) => html.split(needle).length - 1;
const NOW = 100000;

test('an empty display says the aide has shown nothing, and has no parts or earlier topics', () => {
  const html = aideDisplayHtml(aideEmptyDisplay());
  assert.ok(html.includes('副官はまだ何も出していません'));
  assert.equal(count(html, 'aide-part '), 0);
  assert.ok(!html.includes('前の話題'));
  assert.ok(!html.includes('この話題で出したもの'));
});

test('a display state keeps its parts in the order given, opens only the first and names its topic', () => {
  registerAidePart('gate', p => ({ icon: 'pending_actions', label: '確認待ち', where: 'acme/widget', title: p.ref, body: `<p>${p.id}</p>`, cls: 'wait' }));
  const html = aideDisplayHtml({
    topic: { key: 'acme/widget#p40', label: 'acme/widget · 親 #40 Timer' },
    parts: [{ id: 'p3', kind: 'gate', ref: 'third' }, { id: 'p2', kind: 'gate', ref: 'second' }, { id: 'p1', kind: 'gate', ref: 'first' }],
    earlier: [],
  });
  assert.ok(html.indexOf('data-aide-part="p3"') < html.indexOf('data-aide-part="p2"'));
  assert.ok(html.indexOf('data-aide-part="p2"') < html.indexOf('data-aide-part="p1"'));
  assert.equal(count(html, '<details class="aide-part'), 3);
  assert.equal(count(html, ' open>'), 1);
  assert.ok(/data-aide-part="p3" open>/.test(html));
  assert.ok(html.includes('acme/widget · 親 #40 Timer'));
  assert.ok(html.includes('この話題で出したもの 3'));
  assert.ok(html.includes('aide-part wait'));
});

test('earlier topics are only named, in order, and escaped', () => {
  const html = aideDisplayHtml({ topic: null, parts: [], earlier: [{ key: 'a', label: '<b>x</b>' }, { key: 'b', label: 'y' }] });
  assert.ok(html.includes('前の話題 2'));
  assert.ok(html.includes('&lt;b&gt;x&lt;/b&gt;'));
  assert.ok(!html.includes('<b>x</b>'));
  assert.ok(html.indexOf('x&lt;/b&gt;') < html.indexOf('<li>y</li>'));
});

test('a kind with no renderer, and a renderer that throws, are said not to be drawable and do not throw', () => {
  const unknown = aidePartHtml({ id: 'u', kind: 'never-registered' }, true);
  assert.ok(unknown.includes('この部品はまだ表示できません'));
  assert.ok(unknown.includes('never-registered'));
  registerAidePart('broken', () => { throw new Error('boom'); });
  let html;
  assert.doesNotThrow(() => { html = aidePartHtml({ id: 'b', kind: 'broken' }, false); });
  assert.ok(html.includes('この部品はまだ表示できません'));
  assert.ok(!html.includes(' open'));
});

test("a renderer's head fields are escaped and its body is passed through", () => {
  registerAidePart('xss', () => ({ icon: 'x', label: '<i>l</i>', where: '<u>w</u>', title: '<script>alert(1)</script>', body: '<em>kept</em>', cls: '' }));
  const html = aidePartHtml({ id: 'x', kind: 'xss' }, true);
  assert.ok(!html.includes('<script>'));
  assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt;'));
  assert.ok(html.includes('&lt;i&gt;l&lt;/i&gt;') && html.includes('&lt;u&gt;w&lt;/u&gt;'));
  assert.ok(html.includes('<em>kept</em>'));
});

test('a display state with missing fields is made whole before it is drawn', () => {
  assert.deepEqual(JSON.parse(JSON.stringify(aideDisplayOf(null))), { topic: null, parts: [], earlier: [] });
  assert.deepEqual(JSON.parse(JSON.stringify(aideDisplayOf({ parts: [{ kind: 'gate' }] }))), { topic: null, parts: [{ kind: 'gate', id: '' }], earlier: [] });
});

const gate = (key, since, extra = {}) => ({ kind: 'gate', gate: 'plan', title: `G ${key}`, since, key, ...extra });
const entry = (id, gates, extra = {}) => ({ id, nwo: 'acme/widget', ref: id, task: null, gates, items: [...gates], ...extra });

test('waits are the open gates, oldest first, and a parked task has none', () => {
  const entries = [
    entry('t1', [gate('s/g2', 200), gate('s/g1', 100, { gate: 'question' })], { task: { id: '41', title: 'Timer' } }),
    entry('t2', [gate('s/g3', 50)], { items: [gate('s/g3', 50), { kind: 'parked', reason: 'later', since: 60 }] }),
    entry('t3', [gate('s/g4', 150)], { gateTitle: 'Own gate' }),
  ];
  const waits = aideWaits(entries);
  assert.deepEqual(waits.map(w => w.key), ['s/g1', 's/g4', 's/g2']);
  assert.equal(waits.length, 3);
  assert.equal(waits[0].title, 'Timer');
  assert.equal(waits[0].taskId, '41');
  assert.equal(waits[1].title, 'Own gate');
  assert.deepEqual(aideWaits([]), []);
  assert.deepEqual(aideWaits(null), []);
});

test('the pinned row says there are no waits, or how many and which is next', () => {
  assert.equal(aidePinSub([]), '待ちはありません');
  const waits = [{ key: 'a', kind: 'plan', title: 'Timer' }, { key: 'b', kind: 'question', title: 'Other' }];
  assert.equal(aidePinSub(waits), `判断待ち 2 · 次: ${kindOf('plan')[0]} Timer`);
});

test('the peek counts the waits the pane has not shown, and none before it was shown', () => {
  assert.equal(aidePeekCount(new Set(['a']), [{ key: 'a' }, { key: 'b' }]), 1);
  assert.equal(aidePeekCount(new Set(['a']), [{ key: 'a' }]), 0);
  assert.equal(aidePeekCount(null, [{ key: 'a' }]), 0);
});

test('a request about a row starts with its repository, names a numeric task with # and is one clean line of at most 120', () => {
  const numeric = aideAskRowText({ nwo: 'acme/widget', title: 'Timer', taskId: '41', kindLabel: 'プラン' });
  assert.equal(numeric, '[acme/widget] #41 「Timer」のプランについて教えて');
  assert.ok(aideAskRowText({ nwo: 'acme/widget', title: 'T', taskId: 'WID-7' }).startsWith('[acme/widget] WID-7 「T」'));
  assert.ok(aideAskRowText({ nwo: 'acme/widget', title: 'T', taskId: null }).startsWith('[acme/widget] 「T」'));
  const messy = aideAskRowText({ nwo: 'acme/widget', title: 'a\nb\u001b[31mc\t', taskId: '1' });
  assert.ok(!/[\u0000-\u001f]/.test(messy));
  const long = aideAskRowText({ nwo: 'acme/widget', title: 'x'.repeat(500), taskId: '1', kindLabel: 'プラン' });
  assert.ok([...long].length <= 120);
  assert.ok(long.endsWith('について教えて'));
  assert.equal([...aideCleanText('y'.repeat(300))].length, 120);
  assert.equal(aideScopeText('acme/widget'), '[acme/widget] ');
});

test("the aide's state is said in words, and an old `running` row is not busy", () => {
  const word = view => aideStateOf(view, NOW).word;
  assert.equal(word({ state: 'none' }), 'まだ起動していません');
  assert.equal(word({ state: 'starting' }), '起動しています…');
  assert.equal(word({ state: 'stopped' }), '止まっています');
  assert.equal(word({ state: 'unknown' }), '状態不明');
  const running = agentSession => ({ state: 'running', agentSession });
  const busy = aideStateOf(running({ status: 'running', lastEventAt: NOW - 10 }), NOW);
  assert.deepEqual([busy.word, busy.pill, busy.busy], ['作業中', 'pill-good', true]);
  const stale = aideStateOf(running({ status: 'running', lastEventAt: NOW - 600 }), NOW);
  assert.deepEqual([stale.word, stale.busy], ['待機中', false]);
  assert.equal(aideStateOf(running({ status: 'running' }), NOW).busy, false);
  assert.equal(aideStateOf(running({ status: 'done', pending: 'done' }), NOW).busy, true);
  assert.equal(aideStateOf(running({ status: 'waiting' }), NOW).word, '入力待ち');
  assert.equal(aideStateOf(running(null), NOW).word, '待機中');
});

test('the 新着 line is empty for nothing new and escapes the title it names', () => {
  assert.equal(aideNewLineHtml(null), '');
  assert.equal(aideNewLineHtml({ count: 0, first: null }), '');
  const html = aideNewLineHtml({ count: 3, first: { title: '<img src=x>', what: '確認待ち', key: 's/t"1' } });
  assert.ok(html.includes('新着 3'));
  assert.ok(!html.includes('<img'));
  assert.ok(html.includes('&lt;img src=x&gt;'));
  assert.ok(html.includes('data-aide-row="s/t&quot;1"'));
  assert.ok(html.includes('data-aide-ask-new'));
});

test('the header offers a reset only for an aide that can be reset, and marks the context use', () => {
  const head = (view, o) => aideHeadHtml(view, { now: NOW, ...o });
  const reset = html => html.match(/<button[^>]*data-aide-reset[^>]*>/)[0];
  assert.ok(/\sdisabled/.test(reset(head({ state: 'none', startable: true }))));
  assert.ok(/\sdisabled/.test(reset(head({ state: 'unknown', startable: true }))));
  assert.ok(/\sdisabled/.test(reset(head({ state: 'running', startable: false }))));
  assert.ok(/\sdisabled/.test(reset(head({ state: 'running', startable: true }, { resetting: true }))));
  assert.ok(!/\sdisabled/.test(reset(head({ state: 'running', startable: true }))));
  assert.ok(!/\sdisabled/.test(reset(head({ state: 'stopped', startable: true }))));
  const hot = head({ state: 'running', startable: true, agentSession: { contextPercent: 93 } });
  assert.ok(hot.includes('wk-ctx crit') && hot.includes('93%'));
  assert.ok(head({ state: 'running', startable: true, agentSession: { contextPercent: 72 } }).includes('wk-ctx warn'));
  assert.ok(!head({ state: 'running', startable: true }).includes('aide-ctx'));
});

const SOME_WAITS = [{ key: 'k', kind: 'plan', since: 1, nwo: 'acme/widget', title: 'Timer', taskId: '41' }];

test('while the aide has shown nothing, the waits fill the display area, and the area is empty with none', () => {
  const filled = aideDisplayWithWaits(aideEmptyDisplay(), SOME_WAITS);
  assert.deepEqual(filled.topic, { key: 'waits', label: '判断待ちの一覧' });
  assert.equal(filled.parts.length, 1);
  assert.equal(filled.parts[0].kind, 'waits');
  assert.equal(filled.parts[0].shownAt, null);
  assert.deepEqual(aideDisplayWithWaits(aideEmptyDisplay(), []), aideEmptyDisplay());
  assert.deepEqual(aideDisplayWithWaits(null, undefined), aideEmptyDisplay());
});

test('what the aide has shown is drawn as it is, whatever waits', () => {
  const shown = { topic: { key: 'a', label: 'A' }, parts: [{ id: 'p0', kind: 'gate', ref: { board: 'b', id: 'g' }, topic: 'a', shownAt: 5, by: 'aide' }], earlier: [{ key: 'z', label: 'Z' }] };
  assert.deepEqual(aideDisplayWithWaits(shown, SOME_WAITS), aideDisplayOf(shown));
});

test('the earlier topics stay when the waits fill an area with no topic', () => {
  const filled = aideDisplayWithWaits({ topic: null, parts: [], earlier: [{ key: 'z', label: 'Z' }] }, SOME_WAITS);
  assert.deepEqual(filled.earlier, [{ key: 'z', label: 'Z' }]);
});

test('a waits part is drawn, with its time from the page and nothing invalid, whatever shownAt says', () => {
  const html = aideDisplayHtml(aideDisplayWithWaits(aideEmptyDisplay(), SOME_WAITS), { ctx: { waits: SOME_WAITS } });
  assert.ok(!html.includes('この部品はまだ表示できません'));
  assert.ok(html.includes('判断待ちの一覧'));
  assert.ok(!html.includes('Invalid Date'));
});

test('a state as the server sends it goes through the page untouched and is drawn', () => {
  const server = {
    topic: { key: 'acme/widget#p:acme/widget#40', label: 'acme/widget · 親 #40' },
    parts: [{ id: 'p1', kind: 'gate', ref: { board: 'acme-widget-0123', id: 'g1' }, topic: 'acme/widget#p:acme/widget#40', shownAt: 1760000000, by: 'aide' },
      { id: 'p0', kind: 'future-kind', ref: null, topic: 'acme/widget#p:acme/widget#40', shownAt: 1759999999, by: 'aide' }],
    earlier: [{ key: 'waits', label: '判断待ちの一覧' }],
    seq: 2,
  };
  const d = aideDisplayOf(server);
  assert.deepEqual(d.parts[0].ref, { board: 'acme-widget-0123', id: 'g1' });
  assert.equal(d.parts[0].shownAt, 1760000000);
  assert.equal(d.parts[0].by, 'aide');
  const html = aideDisplayHtml(server);
  assert.ok(html.includes('data-aide-part="p1"') && html.includes('data-aide-part="p0"'));
  assert.ok(html.includes('前の話題 1'));
  assert.ok(html.includes('この部品はまだ表示できません'), 'the kind this page does not know');
  const nulled = aideDisplayHtml({ ...server, parts: server.parts.map(p => ({ ...p, shownAt: null })) });
  assert.ok(!nulled.includes('Invalid Date'));
});

/* ── the gate and waits parts, in a context of their own: the first tests above replace the `gate` renderer of the shared one ── */

function world() {
  const c = vm.createContext({ document: { addEventListener() {} }, TextEncoder, TextDecoder });
  for (const file of ['util.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'my-work-away.js', 'decide.js', 'aide-display.js']) {
    vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), c, { filename: file });
  }
  // The attributes the panel's cards carry (card-dialog.js): the aide asks for none.
  vm.runInContext("const expandAttrs = (s, g) => ` data-expand=\"${s}\"`; const expandBtnHtml = l => '<button data-expand-open></button>';", c);
  return c;
}
const W = world();
const run = code => vm.runInContext(code, W);
const plain = x => JSON.parse(JSON.stringify(x));
const { aideNextWait, aideDisplayFollowed, aideLocalHolds } = vm.runInContext('({ aideNextWait, aideDisplayFollowed, aideLocalHolds })', W);
const part = (board, id) => ({ id: `page-gate:${board}/${id}`, kind: 'gate', ref: { board, id } });

test('a wait carries the board, parent, ids and entry its gate is on', () => {
  const [w] = aideWaits([entry('s/41', [gate('acme-1/20260922T041233Z-plan', 100)], { task: { id: '41', title: 'Timer', parent: 'p40' } })]);
  assert.equal(w.board, 'acme-1');
  assert.equal(w.gateId, '20260922T041233Z-plan');
  assert.equal(w.id, 'acme-1/20260922T041233Z-plan');
  assert.equal(w.entryId, 's/41');
  assert.equal(w.parent, 'p40');
  assert.equal(aideWaits([entry('x', [gate('b/g', 1)])])[0].parent, null);
});

const wait = (key, since, extra = {}) => ({ key, since, nwo: 'acme/widget', parent: 'p40', taskId: '41', ...extra });

test('the next wait is the oldest of the same repository and parent, never the one just answered', () => {
  const waits = [wait('b/a', 1, { nwo: 'other/repo' }), wait('b/c', 2, { parent: 'p41' }), wait('b/d', 3), wait('b/e', 4)];
  assert.equal(aideNextWait(waits, { key: 'b/x', nwo: 'acme/widget', parent: 'p40', taskId: '41' }).key, 'b/d');
  assert.equal(aideNextWait(waits, { key: 'b/d', nwo: 'acme/widget', parent: 'p40', taskId: '41' }).key, 'b/e');
  assert.equal(aideNextWait([wait('b/d', 3)], { key: 'b/d', nwo: 'acme/widget', parent: 'p40' }), null);
  assert.equal(aideNextWait([], { key: 'b/d', nwo: 'acme/widget', parent: 'p40' }), null);
  assert.equal(aideNextWait(waits, null), null);
});

test('with no parent the next wait is one of the same task, and a task with no id has none', () => {
  const waits = [wait('b/a', 1, { parent: null, taskId: '7' }), wait('b/b', 2, { parent: null, taskId: '41' }), wait('b/c', 3, { parent: 'p40', taskId: '41' })];
  assert.equal(aideNextWait(waits, { key: 'b/x', nwo: 'acme/widget', parent: null, taskId: '41' }).key, 'b/b');
  assert.equal(aideNextWait(waits, { key: 'b/x', nwo: 'acme/widget', parent: null, taskId: null }), null);
  // A wait with a parent is not of a task with none, even on the same task id.
  assert.equal(aideNextWait([wait('b/c', 3, { parent: 'p40', taskId: '41' })], { key: 'b/x', nwo: 'acme/widget', parent: null, taskId: '41' }), null);
});

test("a part the page made goes on top of the server's, and is dropped as soon as the server's newest part is another", () => {
  const local = { part: part('b', 'g'), serverTop: 'p1', topic: { key: 'page', label: 'L' } };
  const server = { topic: { key: 'a', label: 'A' }, parts: [{ id: 'p1', kind: 'gate' }, { id: 'p0', kind: 'gate' }], earlier: [] };
  const on = plain(aideDisplayFollowed(server, local));
  assert.deepEqual(on.parts.map(p => p.id), ['page-gate:b/g', 'p1', 'p0']);
  assert.equal(on.topic.key, 'a');
  // The aide showed something new.
  assert.deepEqual(plain(aideDisplayFollowed({ ...server, parts: [{ id: 'p2', kind: 'gate' }, ...server.parts] }, local).parts.map(p => p.id)), ['p2', 'p1', 'p0']);
  assert.equal(aideLocalHolds(server, local), true);
  assert.equal(aideLocalHolds({ ...server, parts: [{ id: 'p2' }, ...server.parts] }, local), false);
  assert.equal(aideDisplayFollowed(server, null).parts.length, 2);
});

test('with nothing from the server the page-made part stays until the server shows something, and brings its own topic', () => {
  const local = { part: part('b', 'g'), serverTop: null, topic: { key: 'page', label: 'L' } };
  const none = aideEmptyDisplay();
  const on = plain(aideDisplayFollowed(none, local));
  assert.deepEqual(on.parts.map(p => p.id), ['page-gate:b/g']);
  assert.equal(on.topic.label, 'L');
  // The list of waits does not take its place.
  assert.equal(aideDisplayWithWaits(aideDisplayFollowed(none, local), SOME_WAITS).parts[0].kind, 'gate');
  // null -> an id: the aide has the floor.
  const shown = { topic: { key: 'a', label: 'A' }, parts: [{ id: 'p0', kind: 'gate' }], earlier: [] };
  assert.deepEqual(plain(aideDisplayFollowed(shown, local).parts.map(p => p.id)), ['p0']);
  assert.equal(aideLocalHolds(shown, local), false);
});

const DIFF = [
  'diff --git a/src/a.rs b/src/a.rs', '--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,2 +1,2 @@', ' keep', '-old <b>', '+new <b>',
  'diff --git a/src/b.rs b/src/b.rs', '--- a/src/b.rs', '+++ b/src/b.rs', '@@ -5,2 +5,2 @@', ' ctx b', '-gone', '+here',
].join('\n');

test('the evidence is the hunk of the file a finding names, else the first hunk, capped, and escaped', () => {
  const named = run('aideEvidence')(DIFF, [{ location: 'src/b.rs:6', text: 't' }]);
  assert.ok(named.includes('src/b.rs') && named.includes('here'));
  assert.ok(!named.includes('src/a.rs'));
  const fallback = run('aideEvidence')(DIFF, [{ location: 'elsewhere.rs:1' }]);
  assert.ok(fallback.includes('src/a.rs') && !fallback.includes('src/b.rs'));
  assert.equal(run('aideEvidence')(DIFF, undefined), fallback);
  // Escaped: the diff's own markup is text.
  assert.ok(fallback.includes('old &lt;b&gt;') && !fallback.includes('<b>'));
  const lines = ['diff --git a/big b/big', '--- a/big', '+++ b/big', '@@ -1,30 +1,30 @@', ...Array.from({ length: 30 }, (_, i) => ` line ${i}`)].join('\n');
  assert.equal(run('aideEvidence')(lines, []).split('class="prd-l ').length - 1, 12);
  assert.equal(run('aideEvidence')(lines, [], null, 5).split('class="prd-l ').length - 1, 5);
  assert.equal(run('aideEvidence')('', []), '');
  assert.equal(run('aideEvidence')(null, null), '');
  assert.ok(!fallback.includes('data-prd-plus'), 'no ＋ button of the diff view');
});

const READ = { sha: 'abcdef0123456789', three: { purpose: 'p <b>', approach: 'a', impact: 'i', essence: 'e' },
  points: [{ level: 'attention', path: 'src/a.rs', line: 2, title: 'Point <one>', why: 'because <x>' }, { level: 'check', path: 'src/b.rs', line: 6, title: 'Point two', why: 'w2' }, { level: 'info', path: 'src/a.rs', title: 'Point three', why: 'w3' }],
  annotations: [{ path: 'src/b.rs', kind: 'ex', anchor: '+here', text: 'Explains <here>' }, { path: 'src/b.rs', kind: 'ex', anchor: 'nowhere at all', text: 'Lost one' }, { path: 'src/zzz.rs', kind: 'ex', anchor: 'x', text: 'Orphan' }] };

test('the evidence of a gate has the reader annotations under their lines when the PR has a read-through, and drops what has no line', () => {
  const plainHtml = run('aideEvidenceParts')(DIFF, [{ location: 'src/b.rs:6' }]);
  assert.equal(plainHtml.annotated, false);
  const got = run('aideEvidenceParts')(DIFF, [{ location: 'src/b.rs:6' }], READ);
  assert.equal(got.annotated, true);
  assert.ok(got.html.includes('Explains &lt;here&gt;') && !got.html.includes('Explains <here>'));
  assert.ok(!got.html.includes('Lost one') && !got.html.includes('Orphan'));
  assert.ok(!got.html.includes('data-prd-todraft') && !got.html.includes('data-prd-ask') && !got.html.includes('prd-plus'));
  assert.ok(got.html.indexOf('+') >= 0 && got.html.indexOf('here') < got.html.indexOf('Explains'));
  const html = gatePart({ gate: () => ({ g: GATE, task: TASK, nwo: 'acme/widget', board: 's', read: READ }) });
  assert.ok(html.includes('AI 下読みの注釈（abcdef0 時点）') && html.includes('Explains &lt;here&gt;'));
  assert.ok(!gatePart({ gate: () => ({ g: GATE, task: TASK, nwo: 'acme/widget', board: 's' }) }).includes('AI 下読みの注釈'));
});

test('finished work is what ended, newest first, with hubs left out', () => {
  const j = (kind, since, extra = {}) => ({ entry: { nwo: 'acme/widget', ref: 'r', task: { id: '1', title: 'T' }, isHub: false, ...extra }, live: [{ kind, since }] });
  const judged = new Map([['a', j('done', 10)], ['b', j('failed', 30)], ['c', j('done', 20, { isHub: true })], ['d', j('gate', 40)], ['e', { entry: { nwo: 'x/y', ref: 'r', isHub: false }, live: [] }]]);
  const got = run('aideFinished')(judged);
  assert.deepEqual(got.map(f => [f.entryId, f.kind]), [['b', 'failed'], ['a', 'done']]);
  assert.deepEqual(run('aideFinished')(null), []);
});

const GATE = {
  id: 'g1', kind: 'plan', title: 'Design <review>', focus: 'Pick the **TTL** <x>', unsure: 'Not <sure>', problem: 'Slow <back>', openedAt: '20260922T041233Z',
  options: ['approve', 'changes'], wait: true, task: '41', diff: DIFF, findings: [{ location: 'src/b.rs:6' }],
  choices: [{ id: 'const', label: 'A <const>', why: 'w', points: ['p <1>'], recommended: true, comment: 'Go with A <now>' }, { id: 'cfg', label: 'B' }],
};
const TASK = { id: '41', title: 'Timer <task>', body: '# Heading\n\nA request <b>bold</b> for the timer.\nSecond line.\nThird line is left out.', _slug: 's' };
const gatePart = ctx => run('aidePartHtml')(part('s', 'g1'), true, ctx);

test('a gate part has its sections, escapes what it shows and marks the recommended choice and the draft', () => {
  const html = gatePart({ gate: ref => ({ g: GATE, task: TASK, nwo: 'acme/widget', board: 's', rowKey: 's/41', away: '<ol class="timeline"><li>away</li></ol>', parentTitle: 'Parent <p>' }), draft: () => 'typed <t>' });
  for (const sec of ['このタスク', '前回見てから', '決めること', '迷っていること', '根拠']) assert.ok(html.includes(sec), sec);
  assert.ok(html.includes('data-aide-gate="s/g1"'));
  assert.ok(!html.includes('<review>') && !html.includes('<task>') && !html.includes('<sure>') && !html.includes('<const>') && !html.includes('<now>') && !html.includes('<t>'));
  assert.ok(html.includes('Timer &lt;task&gt;') && html.includes('Go with A &lt;now&gt;') && html.includes('typed &lt;t&gt;'));
  assert.ok(html.includes('AI の推し案'));
  assert.ok(html.includes('data-choice="const"') && html.includes('data-choice="cfg"'));
  assert.ok(html.includes('class="gate-comment"'));
  assert.ok(html.includes('data-act="approve"') && html.includes('data-act="changes"'));
  assert.ok(html.includes('Parent &lt;p&gt;'));
  assert.ok(!html.includes('data-expand-open'), 'no 拡大 button of the panel');
  // 1-2 lines of the request, not the third, not the heading marker.
  assert.ok(html.includes('Heading A request &lt;b&gt;bold&lt;/b&gt; for the timer.'));
  assert.ok(!html.includes('Third line'));
  assert.ok(html.includes('data-aide-row="s/41"') && html.includes('ターミナルを見る'));
  assert.ok(html.includes('data-aide-park') && html.includes('data-aide-expand'));
  assert.ok(html.includes('一覧へ戻る') && html.includes('data-aide-waits'), 'a way back to the list from a live gate');
  assert.ok(html.includes('拡大 → 依頼と計画'));
  assert.ok(html.includes('>here<'));
});

test('a gate that is not a plan says 拡大で開く, and one with no request falls back to its problem, then its title', () => {
  const view = (g, task) => gatePart({ gate: () => ({ g, task, nwo: 'acme/widget', board: 's' }) });
  assert.ok(view({ ...GATE, kind: 'diff' }, TASK).includes('拡大で開く'));
  assert.ok(view(GATE, null).includes('Slow &lt;back&gt;'));
  assert.ok(view({ ...GATE, problem: '' }, null).includes('Design &lt;review&gt;'));
  assert.ok(!view(GATE, null).includes('data-aide-park'), 'no task, nothing to park');
});

test('a gate part says it is loading, failed, or no longer waiting, each in its own way', () => {
  const loading = gatePart({ gate: () => null });
  assert.ok(loading.includes('読み込み中…') && !loading.includes('data-aide-gate'));
  const failed = gatePart({ gate: () => ({ error: 'boom <x>', nwo: 'acme/widget', rowKey: 's/41' }) });
  assert.ok(failed.includes('boom &lt;x&gt;') && failed.includes('ターミナルを見る') && failed.includes('data-aide-row="s/41"'));
  const gone = gatePart({ gate: () => ({ gone: true, nwo: 'acme/widget' }) });
  assert.ok(gone.includes('判断待ちの一覧へ') && gone.includes('data-aide-waits') && !gone.includes('data-act='));
  assert.ok(gatePart({ gate: () => ({ g: { ...GATE, wait: false }, task: TASK }) }).includes('data-aide-waits'));
  assert.ok(run('aidePartHtml')({ id: 'x', kind: 'gate', ref: null }, true, {}).includes('読み込み中…'));
});

test('the waits part groups by repository then parent, puts those with no parent last, and lists what finished', () => {
  const waits = [
    wait('b/1', 10, { title: 'First <t>', kind: 'plan', parent: null }),
    wait('b/2', 20, { title: 'Second', kind: 'question', parent: 'p40' }),
    wait('b/3', 30, { title: 'Third', kind: 'diff', parent: 'p41', nwo: 'other/repo' }),
    wait('b/4', 40, { title: 'Fourth', kind: 'plan', parent: 'p40' }),
  ];
  const html = run('aidePartHtml')({ id: 'page-waits', kind: 'waits', ref: null }, true, {
    waits, finished: [{ entryId: 'e1', nwo: 'acme/widget', kind: 'done', since: 5, title: 'Done <d>' }, { entryId: 'e2', nwo: 'acme/widget', kind: 'failed', since: 6, title: 'Bad' }],
    parentTitle: (nwo, key) => (key === 'p40' ? 'Parent <40>' : ''), repoName: x => x.split('/')[1], rowKey: id => (id === 'e1' ? 's/e1' : null),
    focusOf: w => (w.key === 'b/2' ? { state: 'ok', focus: 'Decide **this** first\nand more' } : { state: 'ok', focus: '' }),
  });
  assert.ok(html.indexOf('widget') < html.indexOf('>repo<'));
  // Within acme/widget: the parent with a title first (oldest wait is the parentless one, but it goes last).
  assert.ok(html.indexOf('Parent &lt;40&gt;') < html.indexOf('親なし'));
  assert.ok(html.indexOf('data-aide-open-gate="b/2"') < html.indexOf('data-aide-open-gate="b/4"'));
  assert.ok(html.indexOf('data-aide-open-gate="b/4"') < html.indexOf('data-aide-open-gate="b/1"'));
  assert.ok(html.includes('>p41<'), 'a parent with no title shows its key');
  assert.ok(html.includes('First &lt;t&gt;') && html.includes('Done &lt;d&gt;') && !html.includes('<d>'));
  assert.ok(html.includes('Decide this first') && !html.includes('and more'));
  assert.equal((html.match(/data-aide-open-gate="/g) || []).length, 4);
  assert.ok(html.includes('離れている間に終わったこと'));
  assert.equal((html.match(/ターミナルを見る/g) || []).length, 1);
  assert.ok(html.includes('失敗') && html.includes('完了'));
  assert.ok(run('aidePartHtml')({ id: 'w', kind: 'waits' }, true, {}).includes('判断待ちはありません'));
});

test('a wait says apart whether its question is still being read, could not be read, is gone or was never written', () => {
  const states = { 'b/1': { state: 'loading' }, 'b/2': { state: 'error', error: 'x' }, 'b/3': { state: 'gone' }, 'b/4': { state: 'ok', focus: '' }, 'b/5': { state: 'ok', focus: 'Ask <this>' } };
  const waits = Object.keys(states).map((k, i) => wait(k, i, { title: k }));
  const html = run('aidePartHtml')({ id: 'w', kind: 'waits' }, true, { waits, focusOf: w => states[w.key] });
  for (const text of ['読み込み中…', '確認内容を読み込めませんでした', 'もう待っていないようです', '確認内容は書かれていません', 'Ask &lt;this&gt;']) assert.ok(html.includes(text), text);
  assert.equal((html.match(/aide-wait-focus/g) || []).length, 5);
});

test('what a decision sends is the typed comment, else the picked choice draft, else the words of a close', () => {
  const g = { choices: [{ id: 'a', comment: 'draft A' }, { id: 'b' }] };
  const sent = run('sentComment');
  assert.equal(sent(g, 'choice', 'a', 'typed'), 'typed');
  assert.equal(sent(g, 'choice', 'a', ''), 'draft A');
  assert.equal(sent(g, 'choice', 'b', ''), '');
  assert.equal(sent(g, 'approve', undefined, ''), '');
  assert.equal(sent(g, 'close', undefined, ''), 'タブで解決済み');
  assert.equal(sent(g, 'close', undefined, 'why'), 'why');
});

/* answerGate: the one place an answer leaves the page, which must work with no board `state` to look in. */
const decideSrc = fs.readFileSync(path.join(__dirname, '..', 'decide.js'), 'utf8');
const fnSrc = name => {
  const m = decideSrc.match(new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));
  assert.ok(m, name);
  return m[0];
};

function answerWorld() {
  const posts = [];
  const notes = [];
  const c = vm.createContext({
    get state() { throw new Error('answerGate must not read state'); },
    posts, notes, gateAnswered(g, key) { notes.push(['answered', key]); },
    gateKey: g => `${g._slug}/${g.id}`, baseOf: x => x._base, baseName: x => x, handedNote: () => '',
    note: (...a) => notes.push(a), boardApi: async (base, url, o) => { posts.push([base, url, JSON.parse(o.body)]); return {}; },
  });
  vm.runInContext([decideSrc.match(/^const sentComment = [\s\S]*?;\n/m)[0], fnSrc('outboxNote'), fnSrc('answerGate'), fnSrc('closeGateOf')].join('\n'), c);
  return { c, posts, notes };
}
const AGATE = { id: 'g1', wait: true, worktree: '/w', _slug: 'acme-1', _base: '/b/acme-1', choices: [{ id: 'const', comment: 'draft A' }, { id: 'cfg' }] };

test("answerGate posts to the gate's own board without looking at `state`, and a choice with a draft sends it when nothing was typed", async () => {
  const w = answerWorld();
  w.c.g = AGATE;
  assert.equal(await vm.runInContext(`answerGate(g, 'approve', undefined, 'typed')`, w.c), true);
  assert.deepEqual(plain(w.posts[0]), ['/b/acme-1', '/api/gates/g1', { decision: 'approve', comment: 'typed' }]);
  await vm.runInContext(`answerGate(g, 'choice', 'const', '')`, w.c);
  assert.equal(w.posts[1][2].comment, 'draft A');
  assert.equal(w.posts[1][2].choice, 'const');
  await vm.runInContext(`answerGate(g, 'choice', 'const', 'mine')`, w.c);
  assert.equal(w.posts[2][2].comment, 'mine');
  await vm.runInContext(`answerGate(g, 'choice', 'cfg', '')`, w.c);
  assert.equal(w.posts[3][2].comment, '');
  assert.ok(w.notes.some(n => n[0] === 'answered' && n[1] === 'acme-1/g1'));
  assert.equal(await vm.runInContext(`closeGateOf(g, 'why')`, w.c), true);
  assert.deepEqual(plain(w.posts[4]), ['/b/acme-1', '/api/gates/g1', { decision: 'close', comment: 'why' }]);
});

test('answerGate sends nothing for a record sent back with no reason, and says false when the board refuses', async () => {
  const w = answerWorld();
  w.c.record = { ...AGATE, wait: false };
  assert.equal(await vm.runInContext(`answerGate(record, 'changes', undefined, '')`, w.c), false);
  assert.equal(w.posts.length, 0);
  w.c.boardApi = async () => { throw new Error('refused'); };
  w.c.g = AGATE;
  assert.equal(await vm.runInContext(`answerGate(g, 'approve', undefined, '')`, w.c), false);
  assert.equal(await vm.runInContext(`closeGateOf(g, '')`, w.c), false);
});

test('a finished entry with no task is named by the list, not by its raw ref', () => {
  const judged = new Map([['s/session:worker-x', { entry: { nwo: 'a/b', ref: 'session:worker-x', task: null, isHub: false }, live: [{ kind: 'done', since: 1 }] }],
    ['s/7', { entry: { nwo: 'a/b', ref: '7', task: { id: '7', title: 'Task seven' }, isHub: false }, live: [{ kind: 'failed', since: 2 }] }]]);
  const got = run('aideFinished')(judged, id => (id === 's/session:worker-x' ? 'worker x (feature)' : ''));
  assert.deepEqual(got.map(f => f.title), ['Task seven', 'worker x (feature)']);
  // Without a namer the ref is all there is.
  assert.equal(run('aideFinished')(judged).find(f => f.entryId === 's/session:worker-x').title, 'session:worker-x');
});

test('a part of a gate that is gone still names its gate in the folded head', () => {
  const head = got => run('aidePartHtml')(part('s', 'g1'), false, { gate: () => got }).match(/aide-part-title">([^<]*)</)[1];
  assert.equal(head({ gone: true, title: 'Design <review>' }), 'Design &lt;review&gt;');
  assert.equal(head({ gone: true }), 'g1');
  assert.equal(head({ error: 'x' }), 'g1');
});

/* ── read, task, pr, session and diff ── */

const partOf = (kind, ref) => ({ id: `p-${kind}`, kind, ref });
const html = (kind, ref, ctx) => run('aidePartHtml')(partOf(kind, ref), true, ctx);
const DIFF_FILES = () => run('prParseDiff')(DIFF);

test('a read-through opens the point it names, or the ones of a level, else what needs attention, else the first', () => {
  const pick = ref => plain(run('aideReadPick')(READ, ref));
  assert.deepEqual(pick({ pr: 'a/b#1' }), { shown: [0], rest: [1, 2] });
  assert.deepEqual(pick({ pr: 'a/b#1', point: 2 }), { shown: [1], rest: [0, 2] });
  assert.deepEqual(pick({ pr: 'a/b#1', level: 'info' }), { shown: [2], rest: [0, 1] });
  assert.deepEqual(pick({ pr: 'a/b#1', point: 9 }), { shown: [], rest: [0, 1, 2] });
  const calm = { points: [{ level: 'check' }, { level: 'info' }] };
  assert.deepEqual(plain(run('aideReadPick')(calm, {})), { shown: [0], rest: [1] });
  assert.deepEqual(plain(run('aideReadPick')({ points: [] }, {})), { shown: [], rest: [] });
});

const REC = { id: 'acme/widget#7', repo: 'Acme/Widget', number: 7, title: 'Add <timer>', headSha: 'abcdef0123456789', task: { board: 's', id: '41' }, ai: { status: 'ready', sha: 'abcdef0123456789' } };

test('the read part draws the three lines, the picked point with its code, the rest folded, and escapes the reader', () => {
  const diff = { state: 'ok', files: DIFF_FILES() };
  const ctx = { pr: () => ({ state: 'ok', rec: REC, read: READ, diff }) };
  const out = html('read', { board: '', id: '', pr: 'acme/widget#7' }, ctx);
  assert.ok(out.includes('この PR がすること') && out.includes('p &lt;b&gt;') && !out.includes('p <b>'));
  assert.ok(out.includes('Point &lt;one&gt;') && out.includes('because &lt;x&gt;') && out.includes('要注意'));
  assert.ok(out.includes('ほかの見るポイント 2') && out.includes('<details class="aide-more">'));
  // The point has no code in this diff (src/a.rs line 2 is a context line there); the annotated one is on point two.
  const two = html('read', { pr: 'acme/widget#7', point: 2 }, ctx);
  assert.ok(two.includes('Point two') && two.includes('Explains &lt;here&gt;') && two.includes('class="prd-l prd-a'));
  assert.ok(!two.includes('prd-plus') && !two.includes('data-prd-todraft'));
  assert.ok(two.includes('data-aide-open-read') && two.includes('data-aide-pr="acme/widget#7"') && two.includes('data-aide-board="s"') && two.includes('data-aide-task="41"'));
  assert.ok(two.includes('data-aide-ask-reader') && /data-aide-ask-reader[^>]*disabled/.test(two) && two.includes('worker が動いていません'));
  const open = { ...REC, ai: { ...REC.ai, terminal: { backend: 'tmux', window: 'w' } } };
  assert.ok(!/data-aide-ask-reader[^>]*disabled/.test(html('read', { pr: 'x' }, { pr: () => ({ state: 'ok', rec: open, read: READ, diff }) })));
  const stale = html('read', { pr: 'x' }, { pr: () => ({ state: 'ok', rec: { ...REC, headSha: '1111111222' }, read: READ, diff }) });
  assert.ok(stale.includes('abcdef0') && stale.includes('1111111'));
  assert.ok(html('read', { pr: 'x' }, { pr: () => ({ state: 'ok', rec: REC, read: READ, diff: { state: 'loading', files: [] } }) }).includes('差分を読み込み中…'));
});

test('the read part says loading, failed, none, and a split proposal has no points to pick', () => {
  const say = got => html('read', { pr: 'acme/widget#7' }, { pr: () => got });
  assert.ok(say({ state: 'loading' }).includes('読み込み中…'));
  assert.ok(say({ state: 'error', error: 'boom <x>' }).includes('boom &lt;x&gt;'));
  assert.ok(say({ state: 'none', rec: REC }).includes('まだありません'));
  assert.ok(html('read', { pr: 'x' }, {}).includes('読み込み中…'));
  const split = say({ state: 'ok', rec: REC, read: { ...READ, mode: 'split', points: [], overall: { body: 'Split <it>' } }, diff: { state: 'gone', files: [] } });
  assert.ok(split.includes('分割案') && split.includes('Split &lt;it&gt;') && !split.includes('見るポイント'));
});

const TCARD = { id: '41', title: 'Timer <task>', body: 'Do the <b>timer</b>\nsecond', status: 'pr', pr: 'https://github.com/acme/widget/pull/7', prTurn: 'changes' };

test('the task part names the task, its plan and what happens next, with the buttons to open it', () => {
  const plan = { title: 'Plan <A>', state: 'decided', at: '20260922T041233Z', text: 'Goal **is** <x>\nmore' };
  const out = html('task', { board: 's', id: '41' }, { task: () => ({ task: TCARD, plan, gates: [], nwo: 'acme/widget', rowKey: 's/41' }) });
  for (const t of ['このタスク', '計画', '次に起きること']) assert.ok(out.includes(t), t);
  assert.ok(out.includes('Timer &lt;task&gt;') && out.includes('Plan &lt;A&gt;') && out.includes('決定済み') && out.includes('Goal is &lt;x&gt;'));
  assert.ok(out.includes('修正の依頼が来ています'));
  assert.ok(out.includes('data-aide-expand') && out.includes('data-aide-target="request"') && out.includes('data-aide-row="s/41"'));
  assert.ok(!out.includes('<b>timer'));
  const waits = html('task', { board: 's', id: '41' }, { task: () => ({ task: TCARD, plan: null, gates: [{ kind: 'diff', wait: true }], nwo: 'a/b' }) });
  assert.ok(waits.includes('計画はまだありません') && waits.includes('あなたの判断待ち'));
  assert.ok(html('task', { board: 's', id: '41' }, { task: () => null }).includes('読み込み中…'));
  assert.ok(html('task', { board: 's', id: '41' }, { task: () => ({ gone: true }) }).includes('もうボードにありません'));
  assert.ok(html('task', { board: 's', id: '41' }, { task: () => ({ error: 'x <e>' }) }).includes('x &lt;e&gt;'));
});

const CHILDREN = [{ id: '1', hub: 'h', title: 'Done one', progress: 'merged' }, { id: '2', hub: 'h', title: 'Gate <two>', progress: 'pr' },
  { id: '3', hub: 'h', title: 'PR three', progress: 'pr' }, { id: '4', hub: 'h', title: 'Working four', progress: 'working' }, { id: '5', hub: 'h', title: 'Not five', progress: 'not-started' }];

test('a parent sorts its children into merged, waiting on you, in progress and not started, and says whose turn each is', () => {
  const waitsBy = new Map([['h/2', [{ key: 'h/g2', kind: 'diff', title: 'Gate <two>' }]]]);
  const tasks = new Map([['h/3', { prTurn: 'changes', waitsOnPerson: true }], ['h/4', { prTurn: 'checks' }]]);
  const rows = new Map([['h/4', 's/h/4']]);
  const got = plain(run('aideParentBuckets')(CHILDREN, waitsBy, tasks, rows, 7));
  assert.deepEqual(got.items.map(i => i.bucket), ['merged', 'waiting', 'waiting', 'working', 'not-started']);
  assert.deepEqual(got.counts, { merged: 1, done: 0, waiting: 2, working: 1, notStarted: 3 });
  assert.equal(got.total, 7);
  assert.deepEqual(got.items.map(i => i.turn), ['', `あなた（${run('kindOf')('diff')[0]}）`, 'あなた（修正の依頼が来ています）', 'bot・CI', 'hub（着手待ち）']);
  assert.equal(got.items[1].waitKey, 'h/g2');
  assert.equal(got.items[3].rowKey, 's/h/4');
  const turn = (c, w, t, r) => run('aideChildTurn')(c, w, t, r);
  assert.equal(turn({ progress: 'pr' }, [], { prTurn: 'unrequested' }, false), 'レビュアー');
  assert.equal(turn({ progress: 'not-started' }, [], null, true), 'worker');
  assert.equal(turn({ progress: 'working' }, [], null, false), 'worker');
  assert.equal(turn({ progress: 'merged' }, [], null, false), '');
});

test('the next thing of a parent is its oldest wait, else the first not started in stack order, else that it is done', () => {
  const items = [{ bucket: 'merged', title: 'A' }, { bucket: 'not-started', title: 'B <b>' }, { bucket: 'not-started', title: 'C' }];
  assert.equal(run('aideParentNext')(items, [{ title: 'Wait' }]), 'あなたの判断待ち: Wait');
  assert.equal(run('aideParentNext')(items, []), '次に着手: B <b>');
  assert.equal(run('aideParentNext')([{ bucket: 'merged' }], []), 'すべてマージ済み');
  assert.equal(run('aideParentNext')([{ bucket: 'working' }], []), 'worker が進めています');
});

test('the parent part draws a bar with the four counts, a row per child with its button, and escapes titles', () => {
  const waitsBy = new Map([['h/2', [{ key: 'h/g2', kind: 'diff', title: 't' }]]]);
  const built = run('aideParentBuckets')(CHILDREN, waitsBy, new Map(), new Map([['h/4', 's/h/4']]), 5);
  const out = html('task', { board: 's', parent: 'acme/widget#40' }, { parent: () => ({ p: { title: 'Parent <p>', number: 40, url: 'https://github.com/acme/widget/issues/40' }, children: built.items, counts: built.counts, total: built.total, next: '次に着手: Not five', nwo: 'acme/widget' }) });
  assert.ok(out.includes('wk-bar aide-bar') && out.includes('<i class="merged"') && out.includes('<i class="waiting"') && out.includes('<i class="working"') && out.includes('<i class="not-started"'));
  assert.ok(out.includes('マージ 1 / あなた待ち 1 / 進行中 2 / 未着手 1（全 5）'));
  assert.ok(out.includes('Gate &lt;two&gt;') && !out.includes('<two>') && out.includes('Parent &lt;p&gt;'));
  assert.ok(out.includes('data-aide-open-gate="h/g2"') && out.includes('data-aide-row="s/h/4"'));
  assert.ok(out.includes('次に着手: Not five') && out.includes('GitHub で開く'));
  assert.ok(html('task', { board: 's', parent: 'k' }, { parent: () => ({ gone: true }) }).includes('もうボードにありません'));
});

const RECORD = { id: 'acme/widget#7', repo: 'Acme/Widget', number: 7, url: 'https://github.com/acme/widget/pull/7', title: 'T <t>', draft: false, prState: 'open', state: 'requested',
  additions: 10, deletions: 2, changedFiles: 3, ci: { pass: 4, fail: 0, pending: 1 },
  reviewers: [{ login: 'alice', state: 'approved' }, { login: 'copilot-pull-request-reviewer', state: 'commented' }, { login: 'ci[bot]', state: 'commented' }],
  ai: { status: 'ready', sha: 'abcdef0123456789', counts: { attention: 1, check: 2 } }, myReview: { state: 'commented', submittedAt: '20260922T041233Z' } };

test('the PR part says whose turn it is, the bots, the CI, whether it can merge and what stands in the way, the size and whether it was read', () => {
  const facts = got => html('pr', { board: '', id: '', pr: 'acme/widget#7' }, { prFacts: () => ({ nowSecs: 1e10, ...got }) });
  const out = facts({ rec: RECORD });
  for (const t of ['レビュー（誰の番）', 'レビュー bot', 'CI', 'マージ', '差分の大きさ', '読んだか']) assert.ok(out.includes(t), t);
  assert.ok(out.includes('あなたの番（レビューを頼まれています）'));
  assert.ok(out.includes('copilot-pull-request-reviewer') && out.includes('ci[bot]') && !/レビュー bot<\/span><strong>[^<]*alice/.test(out));
  assert.ok(out.includes('成功 4 / 失敗 0 / 実行中 1') && out.includes('+10') && out.includes('3 files'));
  assert.ok(out.includes('CI が実行中です') && out.includes('コンフリクトは記録にありません（GitHub で確認）'));
  assert.ok(out.includes('下読み済み') && out.includes('abcdef0') && out.includes('あなたのレビュー'));
  assert.ok(out.includes('GitHub で開く') && out.includes('data-aide-open-read'));
  assert.ok(out.includes('T &lt;t&gt;') && !out.includes('<t>'));
  const merge = facts({ rec: { ...RECORD, ci: { pass: 5, fail: 0, pending: 0 } }, task: TCARD, prTurn: 'merge', phase: 'pr-bots' });
  assert.ok(merge.includes('承認済み・CI 成功') && !merge.includes('できます') && merge.includes('コンフリクトは記録にありません') && merge.includes('worker が bot の指摘待ち') && merge.includes('承認済みです'));
  const bad = facts({ rec: { ...RECORD, draft: true, ci: { pass: 0, fail: 2, pending: 0 } }, task: TCARD, prTurn: 'draft' });
  assert.ok(bad.includes('下書きです') && bad.includes('CI が落ちています'));
  // A task's PR with no record: only what the task card read of it.
  const norec = facts({ rec: null, task: TCARD, prStatus: { state: 'open', title: 'S <s>', ci: { pass: 1, fail: 0, pending: 0 } }, prTurn: 'changes', prId: 'acme/widget#7' });
  assert.ok(norec.includes('記録がありません') && norec.includes('修正の依頼が来ています') && norec.includes('S &lt;s&gt;'));
  assert.ok(!norec.includes('data-aide-open-read'));
  assert.ok(facts({ rec: null }).includes('この PR は読めません'));
  assert.ok(facts({ loading: true }).includes('読み込み中…') && facts({ error: 'x <e>' }).includes('x &lt;e&gt;'));
});

test('a review bot is told by its login with or without [bot], and a person is not one', () => {
  const bot = run('aideIsBot');
  for (const l of ['dependabot[bot]', 'copilot-pull-request-reviewer', 'coderabbitai', 'github-actions', 'my-bot']) assert.equal(bot(l), true, l);
  for (const l of ['alice', 'bob-smith', 'abbott', '', null]) assert.equal(bot(l), false, String(l));
});

const SROW = (extra = {}, s = {}) => ({ key: 's/41', st: 'working', task: { id: '41', title: 'Timer <task>', pr: '#7' }, data: { now: 1000 },
  s: { agent: 'claude', branch: 'feat/x', present: true, agentSession: { model: 'opus', contextPercent: 42, activity: 'Editing <f>', lastMessage: '**Done** the <thing>\nmore', request: 'Bash' }, ...s }, ...extra });
const FNS = { glyph: st => `<i class="g ${st}"></i>`, label: st => ({ working: '作業中', stopped: '停止', failed: 'エラー', permission: '入力待ち' }[st] || st), why: () => 'why <w>',
  request: s => `許可: ${s.agentSession.request}`, said: m => String(m).split('\n')[0].replace(/\*/g, ''), phaseMinutes: () => 90, title: r => r.task?.title };

test('the session part says the state, the agent and the last message, and whether it is stuck: stopped, waiting for you, one phase too long or not', () => {
  const sess = (r, stuckAfter = 60) => html('session', { board: 's', id: 'a' }, { session: () => ({ r, fns: FNS, nwo: 'acme/widget', stuckAfter }) });
  const out = sess(SROW());
  assert.ok(out.includes('作業中') && out.includes('claude · opus · コンテキスト 42%') && out.includes('Editing &lt;f&gt;'));
  assert.ok(out.includes('feat/x') && out.includes('Timer &lt;task&gt;') && !out.includes('<task>'));
  assert.ok(out.includes('最後のメッセージ — Done the &lt;thing&gt;') && out.includes('<details class="aide-more">'));
  assert.ok(out.includes('ターミナルを見る') && out.includes('data-aide-row="s/41"'));
  assert.ok(out.includes('1時間 同じ工程') && out.includes('aide-stuck'), 'a phase kept longer than the board allows');
  assert.ok(sess(SROW(), 0).includes('止まっていません'));
  assert.ok(sess(SROW({ st: 'stopped' })).includes('停止 — why &lt;w&gt;'));
  assert.ok(sess(SROW({ st: 'permission' })).includes('入力待ち（許可や答えを待っています）') && sess(SROW({ st: 'permission' })).includes('許可: Bash'));
  assert.ok(!sess(SROW({ key: null })).includes('ターミナルを見る'));
  assert.ok(html('session', { board: 's', id: 'a' }, { session: () => ({ gone: true }) }).includes('いまの一覧にありません'));
  assert.ok(html('session', { board: 's', id: 'a' }, { session: () => null }).includes('読み込み中…'));
});

test('the diff excerpt is the lines of a side, with the removed lines between and a header again across hunks', () => {
  const [a, b] = DIFF_FILES();
  const ex = (file, r) => plain(run('prDiffExcerpt')(file, r));
  assert.deepEqual(ex(a, { from: 2, to: 2 }).ks, [2]);
  assert.deepEqual([...run('prDiffExcerpt')(a, { from: 2, to: 2 }).hunks], [0]);
  // The right side 1..2 takes the context line and the added one, and the removed line between them.
  assert.deepEqual(ex(a, { from: 1, to: 2 }).ks, [0, 1, 2]);
  // The left side: the context line and the removed one, not the added.
  assert.deepEqual(ex(a, { from: 1, to: 2, side: 'left' }).ks, [0, 1]);
  assert.equal(ex(a, { from: 50, to: 60 }), null);
  const two = ['diff --git a/x b/x', '--- a/x', '+++ b/x', '@@ -1,2 +1,2 @@', ' one', '-two', '+TWO', '@@ -10,2 +10,2 @@', ' ten', '-eleven', '+ELEVEN'].join('\n');
  const [x] = run('prParseDiff')(two);
  const got = run('prDiffExcerpt')(x, { from: 2, to: 10 });
  const out = run('prDiffExcerptHtml')(x, got.ks, null, null);
  assert.equal(out.split('class="prd-h"').length - 1, 2, 'the second hunk header is drawn where the range crosses into it');
  assert.ok(out.indexOf('@@ -1,2') < out.indexOf('@@ -10,2'));
  void b;
});

test('the diff part draws the range of a PR diff with its annotations, or says the range is not in the diff, or the source is gone', () => {
  const [, b] = DIFF_FILES();
  const res = run('prDiffResolve')(DIFF_FILES(), READ).files.get('src/b.rs');
  const part = { pr: 'acme/widget#7', path: 'src/b.rs', from: 5, to: 6, side: 'right' };
  const out = html('diff', part, { diffOf: () => ({ state: 'ok', file: DIFF_FILES()[1], res, read: READ, board: 's', taskId: '41' }) });
  assert.ok(out.includes('src/b.rs:5-6') && out.includes('ctx b') && out.includes('here') && out.includes('Explains &lt;here&gt;'));
  assert.ok(out.includes('data-aide-open-read') && out.includes('data-aide-pr="acme/widget#7"'));
  assert.ok(html('diff', { ...part, from: 80, to: 90 }, { diffOf: () => ({ state: 'ok', file: b, res: null }) }).includes('この範囲は差分にありません'));
  assert.ok(html('diff', { ...part, side: 'left' }, { diffOf: () => ({ state: 'ok', file: b }) }).includes('（削除された側）'));
  const gate = { board: 's', id: 'g1', path: 'src/b.rs', from: 5, to: 6 };
  const g = html('diff', gate, { diffOf: () => ({ state: 'ok', file: b, board: 's', taskId: '41', gateRef: 's/g1' }) });
  assert.ok(g.includes('拡大で開く') && g.includes('data-aide-gate-ref="s/g1"') && !g.includes('data-aide-open-read'));
  assert.ok(html('diff', gate, { diffOf: () => ({ state: 'gone' }) }).includes('この判断待ちの差分はもう読めません'));
  assert.ok(html('diff', part, { diffOf: () => ({ state: 'gone' }) }).includes('この PR の差分は、もう読めません'));
  assert.ok(html('diff', part, { diffOf: () => ({ state: 'error', error: 'x <e>' }) }).includes('x &lt;e&gt;'));
  assert.ok(html('diff', part, {}).includes('読み込み中…'));
});

test('a part of a kind a newer server sends, and one whose renderer throws, is a short line and the area stays', () => {
  const area = run('aideDisplayHtml')({ topic: { key: 'k', label: 'L' }, parts: [{ id: 'p2', kind: 'hologram', ref: { board: '', id: '' } }, { id: 'p1', kind: 'pr', ref: { board: '', id: '', pr: 'a/b#1' } }], earlier: [] },
    { ctx: { prFacts: () => { throw new Error('boom'); } } });
  assert.ok(area.includes('この部品はまだ表示できません') && area.includes('hologram'));
  assert.equal((area.match(/data-aide-part=/g) || []).length, 2);
});

test('a part whose last read failed says so over what it still shows, and says nothing when the read worked', () => {
  const err = 'boom <x>';
  const diff = { state: 'ok', files: DIFF_FILES() };
  const has = out => out.includes('最新の読み込みに失敗しました: boom &lt;x&gt;') && !out.includes('<x>');
  assert.ok(has(html('read', { pr: 'a/b#1' }, { pr: () => ({ state: 'ok', rec: REC, read: READ, diff, refreshError: err }) })));
  assert.ok(has(html('pr', { pr: 'a/b#1' }, { prFacts: () => ({ rec: RECORD, nowSecs: 1e10, refreshError: err }) })));
  assert.ok(has(html('diff', { pr: 'a/b#1', path: 'src/b.rs', from: 5, to: 6 }, { diffOf: () => ({ state: 'ok', file: DIFF_FILES()[1], refreshError: err }) })));
  assert.ok(has(html('task', { board: 's', id: '41' }, { task: () => ({ task: TCARD, plan: null, gates: [], refreshError: err }) })));
  assert.ok(has(gatePart({ gate: () => ({ g: GATE, task: TASK, nwo: 'a/b', board: 's', refreshError: err }) })));
  assert.ok(!html('read', { pr: 'a/b#1' }, { pr: () => ({ state: 'ok', rec: REC, read: READ, diff }) }).includes('最新の読み込みに失敗'));
});

test('a child that finished with no merged PR is done, not merged, and a parent is not said to be all merged while one is only done', () => {
  const kids = [{ id: '1', hub: 'h', title: 'A', progress: 'merged' }, { id: '2', hub: 'h', title: 'B', progress: 'done' }];
  const got = plain(run('aideParentBuckets')(kids, new Map(), new Map(), new Map(), 2));
  assert.deepEqual(got.items.map(i => i.bucket), ['merged', 'done']);
  assert.deepEqual(got.counts, { merged: 1, done: 1, waiting: 0, working: 0, notStarted: 0 });
  assert.equal(run('aideParentNext')(got.items, []), 'すべて終わっています（マージされていないものがあります）');
  assert.equal(run('aideParentNext')([got.items[0]], []), 'すべてマージ済み');
  const out = html('task', { board: 's', parent: 'k' }, { parent: () => ({ p: { title: 'P' }, children: got.items, counts: got.counts, total: 2, next: 'n' }) });
  assert.ok(out.includes('マージ 1 / 完了 1 / あなた待ち 0') && out.includes('<i class="done"') && out.includes('完了'));
});

test('the session part says when the last read of its board failed', () => {
  const out = html('session', { board: 's', id: 'a' }, { session: () => ({ r: SROW(), fns: FNS, nwo: 'a/b', stuckAfter: 0, refreshError: 'boom' }) });
  assert.ok(out.includes('最新の読み込みに失敗しました: boom'));
});

test('a parent says when the last read of the work document failed', () => {
  const out = html('task', { board: 's', parent: 'k' }, { parent: () => ({ p: { title: 'P' }, children: [], counts: { merged: 0, done: 0, waiting: 0, working: 0, notStarted: 0 }, total: 0, next: 'n', refreshError: 'boom' }) });
  assert.ok(out.includes('最新の読み込みに失敗しました: boom'));
});

test('a parent with children the tracker counts and the board does not record is not called all merged', () => {
  const merged = [{ bucket: 'merged', title: 'A' }, { bucket: 'merged', title: 'B' }];
  assert.equal(run('aideParentNext')(merged, [], 2), 'すべてマージ済み');
  assert.equal(run('aideParentNext')(merged, [], 5), '次に着手: まだ記録のない子 3 件');
  assert.equal(run('aideParentNext')([...merged, { bucket: 'not-started', title: 'C' }], [], 5), '次に着手: C');
});

test('a task not handed over, or queued at the hub, is not said to have a worker', () => {
  const next = status => run('aideTaskNext')({ id: '1', status }, []);
  assert.ok(next('backlog').includes('まだ hub に渡していません') && !next('backlog').includes('worker'));
  assert.ok(next('queued').includes('hub が着手するのを待っています'));
  assert.equal(next('dispatched'), 'worker が作業中です');
  assert.equal(next('pr'), 'PR の結果待ちです');
  const turn = (progress, task) => run('aideChildTurn')({ progress }, [], task, false);
  assert.equal(turn('not-started', { status: 'backlog' }), 'hub（着手待ち）');
  assert.equal(turn('not-started', { status: 'queued' }), 'hub（着手待ち）');
  assert.equal(turn('working', { status: 'dispatched' }), 'worker');
  assert.equal(turn('pr', { status: 'pr', prTurn: 'unrequested' }), 'レビュアー');
});

test('where the points are in the diff is found once for a diff and a read-through, and the evidence of a gate is parsed once', () => {
  const files = DIFF_FILES();
  const read = { points: [{ path: 'src/b.rs', line: 6 }, { path: 'src/a.rs' }, { path: 'nowhere.rs', line: 1 }] };
  const first = run('aidePointPlaces');
  const got = first(files, read);
  assert.equal(got[0].k, 2);
  assert.equal(got[1], null);
  assert.equal(got[2], null);
  assert.strictEqual(first(files, read), got, 'the same array is given back, not found again');
  assert.notStrictEqual(first(files, { points: read.points }), got);
  const parse = run('aideParsedGateDiff');
  assert.strictEqual(parse(DIFF), parse(DIFF));
});

test("a finished task's PR is read from the task's own status before a stale record: merged is merged, and its CI is the task's", () => {
  const stale = { ...RECORD, draft: true, state: 'requested', ci: { pass: 1, fail: 0, pending: 2 }, reviewers: [] };
  const out = html('pr', { board: 's', id: '41' }, { prFacts: () => ({ rec: stale, task: TCARD, prStatus: { state: 'merged', title: 'Merged <t>', review: 'approved', ci: { pass: 58, fail: 0, pending: 0 } }, prTurn: null, nowSecs: 1e10 }) });
  assert.ok(out.includes('マージ済みです') && !out.includes('下書きです') && !out.includes('まだです') && !out.includes('コンフリクトは記録にありません'));
  assert.ok(out.includes('成功 58 / 失敗 0 / 実行中 0') && out.includes('Merged &lt;t&gt;'));
  const closed = html('pr', { board: 's', id: '41' }, { prFacts: () => ({ rec: stale, task: TCARD, prStatus: { state: 'closed', ci: {} }, nowSecs: 1e10 }) });
  assert.ok(closed.includes('マージされずに閉じられています') && !closed.includes('下書きです'));
  // An open PR with no live turn: the status says approved, and the record's draft flag is not what is read.
  const open = html('pr', { board: 's', id: '41' }, { prFacts: () => ({ rec: stale, task: TCARD, prStatus: { state: 'open', review: 'approved', ci: { pass: 3, fail: 0, pending: 0 } }, nowSecs: 1e10 }) });
  assert.ok(open.includes('承認済み・CI 成功') && !open.includes('下書きです'));
});

test('a finished task says its plan cannot be read here, not that there is none; a live one with none says there is none', () => {
  const view = status => html('task', { board: 's', id: '41' }, { task: () => ({ task: { ...TCARD, status }, plan: null, gates: [], nwo: 'a/b' }) });
  assert.ok(view('done').includes('終わったタスクの計画はここでは読めません') && !view('done').includes('計画はまだありません'));
  assert.ok(view('cancelled').includes('終わったタスクの計画はここでは読めません'));
  assert.ok(view('dispatched').includes('計画はまだありません'));
});

test('read parts of one PR are told apart by their pick in the heading', () => {
  const head = ref => html('read', { pr: 'a/b#1', ...ref }, { pr: () => ({ state: 'loading' }) }).match(/aide-part-kind">.*?<\/span>(.*?)<\/span>/)[1];
  assert.equal(head({}), 'AI 下読み');
  assert.equal(head({ point: 3 }), 'AI 下読み · 見るポイント 3');
  assert.equal(head({ level: 'info' }), 'AI 下読み · 参考のポイント');
  assert.equal(head({ level: 'attention' }), 'AI 下読み · 要注意のポイント');
});

test("a task's PR status is turned as the server turns it, a review still owed being another reviewer's", () => {
  const turn = (state, review, ci = {}) => run('aideTurnOfStatus')({ state, review, ci });
  assert.equal(turn('open', 'required'), 'other-reviewer');
  assert.equal(turn('open', 'none'), 'unrequested');
  assert.equal(turn('open', 'approved'), 'merge');
  assert.equal(turn('open', 'approved', { pending: 1 }), 'checks');
  assert.equal(turn('open', 'required', { fail: 1 }), 'ci-failed');
  assert.equal(turn('open', 'changes', { fail: 1 }), 'changes');
  assert.equal(turn('draft', 'approved'), 'draft');
  assert.equal(turn('merged', 'none'), 'merged');
  assert.equal(turn('weird', 'none'), null);
  const out = html('pr', { board: 's', id: '41' }, { prFacts: () => ({ rec: null, task: TCARD, prStatus: { state: 'open', review: 'required', ci: { pass: 1 } }, nowSecs: 1e10 }) });
  assert.ok(out.includes('他のレビュアーの返事待ちです'));
});

test('two gate diffs shown together are each parsed once, however the redraws alternate between them', () => {
  const parse = run('aideParsedGateDiff');
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

const REVREC = (id, repo, extra = {}) => ({ id, repo, number: Number(id.split('#')[1]), title: `T ${id} <t>`, author: 'alice', state: 'requested', requestedAt: '20261001T000000Z',
  events: { requested: '20261001T000000Z' }, ...extra });

test('the review requests are grouped by repository with their title, author, new or asked again, and where the 下読み stands', () => {
  const records = [
    REVREC('b/y#2', 'B/Y', { rerequest: true, ai: { status: 'ready', sha: 's', counts: { attention: 1, check: 2 } } }),
    REVREC('a/x#9', 'A/X', { requestedAt: '20261003T000000Z' }),
    REVREC('a/x#1', 'A/X', { author: 'bob' }),
    REVREC('a/x#5', 'A/X', { state: 'waiting-on-author' }),
    REVREC('a/x#6', 'A/X', { state: 'done' }),
  ];
  const out = html('reviews', null, { reviews: () => ({ state: 'ok', records, marks: { 'a/x#1': { read: 1e12 } }, nowSecs: 1e10 }) });
  assert.ok(out.indexOf('A/X') < out.indexOf('B/Y'));
  assert.equal((out.match(/data-aide-open-review="/g) || []).length, 3, 'only what waits on the person');
  assert.ok(out.includes('data-aide-open-review="a/x#1"') && !out.includes('a/x#5') && !out.includes('a/x#6'));
  assert.ok(out.indexOf('a/x#1') < out.indexOf('a/x#9'), 'the oldest request first');
  assert.ok(out.includes('T a/x#1 &lt;t&gt;') && !out.includes('<t>') && out.includes('@bob'));
  assert.ok(out.includes('頼み直し') && out.includes('下読み済み · 要注意 1 · 確認 2') && out.includes('AI 下読みはまだです'));
  // Never marked read, with an event after: new; marked read after the event: not new.
  const marks = { 'a/x#9': { read: 0 } };
  const fresh = html('reviews', null, { reviews: () => ({ state: 'ok', records: [records[1]], marks, nowSecs: 1e10 }) });
  assert.ok(fresh.includes('新着'));
  assert.ok(!html('reviews', null, { reviews: () => ({ state: 'ok', records: [records[1]], marks: { 'a/x#9': { read: 1e12 } }, nowSecs: 1e10 }) }).includes('新着'));
  assert.ok(out.includes('3 件'));
});

test('the review requests part says empty, loading and failed, and when its last read failed', () => {
  const say = got => html('reviews', null, { reviews: () => got });
  assert.ok(say({ state: 'ok', records: [] }).includes('レビュー依頼はありません'));
  assert.ok(say({ state: 'ok', records: [REVREC('a/x#1', 'A/X', { state: 'done' })] }).includes('レビュー依頼はありません'));
  assert.ok(say({ state: 'loading' }).includes('読み込み中…'));
  assert.ok(html('reviews', null, {}).includes('読み込み中…'));
  assert.ok(say({ state: 'error', error: 'boom <x>' }).includes('boom &lt;x&gt;'));
  assert.ok(say({ state: 'ok', records: [], refreshError: 'boom' }).includes('最新の読み込みに失敗しました: boom'));
  assert.deepEqual(plain(run('aideReadPart')('a/b#1')).ref, { board: '', id: '', pr: 'a/b#1' });
});

test('the review requests part says how the list was synced, and never reads an unsynced list as no requests', () => {
  const say = (records, sync) => html('reviews', null, { reviews: () => ({ state: 'ok', records, nowSecs: 1e10, sync }) });
  const never = say([], { words: 'まだ同期していません', never: true, stale: false, notes: [] });
  assert.ok(never.includes('まだ同期していません') && never.includes('同期するまで、レビュー依頼があるかは分かりません') && !never.includes('レビュー依頼はありません'));
  const old = say([REVREC('a/x#1', 'A/X')], { words: '同期・最終同期: 2時間前', never: false, stale: true, notes: [{ alert: true, text: '前回の同期に失敗しました: <e>' }, { text: '一部を読めませんでした' }] });
  assert.ok(old.includes('同期・最終同期: 2時間前') && old.includes('aide-sync warn') && old.includes('前回の同期に失敗しました: &lt;e&gt;') && old.includes('一部を読めませんでした'));
  assert.ok(old.includes('data-aide-open-review'));
  assert.ok(say([], { words: '同期・最終同期: たった今', never: false, stale: false, notes: [] }).includes('レビュー依頼はありません'));
});

test('a read part opened from the review requests has the way back to the list, and one the aide showed does not', () => {
  const ctx = { pr: () => ({ state: 'none', rec: REC }) };
  const local = run('aidePartHtml')({ ...run('aideReadPart')('a/b#1') }, true, ctx);
  assert.ok(local.includes('data-aide-reviews') && local.includes('一覧へ戻る'));
  assert.ok(!html('read', { pr: 'a/b#1' }, ctx).includes('data-aide-reviews'));
  const part = plain(run('aideReviewsPart')());
  assert.deepEqual([part.kind, part.ref, part.by], ['reviews', null, 'board']);
  assert.equal(run('AIDE_REVIEWS_TOPIC').key, 'reviews');
});

test('an empty list is said to have no requests only after a clean, fresh sync, else it may have missed some', () => {
  const say = sync => html('reviews', null, { reviews: () => ({ state: 'ok', records: [], nowSecs: 1e10, sync: { words: 'w', never: false, stale: false, notes: [], ...sync } }) });
  assert.ok(say({}).includes('レビュー依頼はありません'));
  for (const sync of [{ stale: true }, { notes: [{ text: '一部を読めませんでした' }] }, { notes: [{ alert: true, text: 'x' }] }]) {
    const out = say(sync);
    assert.ok(out.includes('同期に問題があるので、レビュー依頼が漏れているかもしれません') && !out.includes('レビュー依頼はありません'), JSON.stringify(sync));
  }
  assert.ok(html('reviews', null, { reviews: () => ({ state: 'ok', records: [], sync: { words: 'w', never: true, notes: [] } }) }).includes('同期するまで'));
});


/* ── the request part ── */

const stepsOf = (task, gates = []) => JSON.parse(JSON.stringify(aideRequestSteps(task, gates)));
const stateOf = (steps, key) => steps.find(s => s.key === key)?.state;
const requestTask = (extra = {}) => ({ id: 't1', title: 'Add a retry', body: 'Add a retry to the sync', status: 'queued', needsReading: true, ...extra });

test('a request just sent is sent, and the hub is yet to read it', () => {
  const steps = stepsOf(requestTask());
  assert.deepEqual(steps.map(s => s.key), ['sent', 'confirm', 'issue', 'worker', 'pr']);
  assert.equal(stateOf(steps, 'sent'), 'done');
  assert.equal(stateOf(steps, 'confirm'), 'now');
  assert.ok(steps.find(s => s.key === 'confirm').text.includes('hub が読むのを待っています'));
  assert.equal(count(steps.map(s => s.state).join(' '), 'now'), 1);
  assert.ok(!steps.some(s => s.state === 'failed'));
});

test('an open dispatch gate of the task is the hub waiting for the person to confirm what it read', () => {
  const steps = stepsOf(requestTask({ needsReading: false }), [{ id: 'g1', kind: 'dispatch', wait: true }]);
  assert.equal(stateOf(steps, 'confirm'), 'now');
  assert.ok(steps.find(s => s.key === 'confirm').text.includes('読み取りの確認待ち'));
  // A record of a dispatch gate, or a gate of another kind, is not that.
  const other = stepsOf(requestTask({ needsReading: false }), [{ id: 'g1', kind: 'dispatch', wait: false }, { id: 'g2', kind: 'plan', wait: true }]);
  assert.equal(stateOf(other, 'confirm'), 'done');
});

test('a filed issue names its number, from the issue or its URL, and the worker step follows', () => {
  const byNumber = stepsOf(requestTask({ needsReading: false, issue: '#42' }));
  assert.equal(stateOf(byNumber, 'issue'), 'done');
  assert.ok(byNumber.find(s => s.key === 'issue').text.includes('起票済み #42'));
  assert.equal(stateOf(byNumber, 'worker'), 'now');
  const byUrl = stepsOf(requestTask({ needsReading: false, issueUrl: 'https://github.com/acme/widget/issues/7' }));
  assert.ok(byUrl.find(s => s.key === 'issue').text.includes('#7'));
});

test('a worker that started, and a PR, are done; one that is not there yet is not', () => {
  const started = stepsOf(requestTask({ needsReading: false, issue: '#42', status: 'dispatched', worktree: '/w' }));
  assert.equal(stateOf(started, 'worker'), 'done');
  assert.equal(stateOf(started, 'pr'), 'now');
  const out = stepsOf(requestTask({ needsReading: false, issue: '#42', status: 'pr', worktree: '/w', pr: 'https://github.com/acme/widget/pull/9' }));
  assert.equal(stateOf(out, 'pr'), 'done');
  assert.ok(out.find(s => s.key === 'pr').text.includes('pull/9'));
  assert.ok(!out.some(s => s.state === 'now'));
});

test('a request that could not start, or was cancelled, ends in a failed step and has no step at now', () => {
  const failed = stepsOf(requestTask({ needsReading: false, note: 'Could not start: no worktree name' }));
  assert.equal(failed.at(-1).state, 'failed');
  assert.ok(failed.at(-1).text.includes('着手できず') && failed.at(-1).text.includes('no worktree name'));
  assert.ok(!failed.some(s => s.state === 'now'));
  const cancelled = stepsOf(requestTask({ status: 'cancelled' }));
  assert.equal(cancelled.at(-1).state, 'failed');
  assert.ok(cancelled.at(-1).text.includes('取り消'));
  // An ordinary note is not a failure.
  assert.ok(!stepsOf(requestTask({ note: 'PR指摘: x' })).some(s => s.state === 'failed'));
});

test('the request part says what was sent, to which hub, how far it got, and escapes all of it', () => {
  const ref = { board: 'acme-widget-abc', id: 't1' };
  const got = {
    task: requestTask({ title: '<b>T</b>', body: '<script>x</script> do it' }), gates: [{ id: 'g1', kind: 'dispatch', wait: true }],
    nwo: 'acme/widget', hubLabel: '親 Issue の hub · <i>P</i>', hubPresent: false, hubRowKey: 'acme-widget-abc/hub:h1', rowKey: 'acme-widget-abc/t1', waitKey: 'acme-widget-abc/g1',
  };
  const html = aidePartHtml({ id: 'p1', kind: 'request', ref }, true, { request: () => got });
  assert.ok(html.includes('依頼') && html.includes('送った内容') && html.includes('送り先') && html.includes('進み具合'));
  assert.ok(html.includes('&lt;script&gt;x&lt;/script&gt; do it'));
  assert.ok(!html.includes('<script>') && !html.includes('<b>T</b>') && !html.includes('<i>P</i>'));
  assert.ok(html.includes('hub は止まっています。起動すると読みます'));
  assert.ok(html.includes('data-aide-open-gate="acme-widget-abc/g1"'));
  assert.ok(html.includes('data-aide-row="acme-widget-abc/hub:h1"') && html.includes('hub のターミナル'));
  assert.ok(html.includes('data-aide-row="acme-widget-abc/t1"') && html.includes('worker のターミナル'));
  assert.ok(html.includes('data-aide-expand'));
  // A running hub is not said to be stopped, and a part with no gate or session has none of the buttons.
  const quiet = aidePartHtml({ id: 'p1', kind: 'request', ref }, true, { request: () => ({ ...got, hubPresent: true, hubRowKey: null, rowKey: null, waitKey: null, gates: [] }) });
  assert.ok(!quiet.includes('hub は止まっています') && !quiet.includes('data-aide-open-gate') && !quiet.includes('data-aide-row'));
});

test('a request part says it is loading, could not be read, or has no task any more', () => {
  const ref = { board: 'b', id: 't1' };
  const draw = view => aidePartHtml({ id: 'p1', kind: 'request', ref }, true, { request: () => view });
  assert.ok(draw(null).includes('読み込み中'));
  assert.ok(draw({ error: 'boom <x>' }).includes('読めませんでした: boom &lt;x&gt;'));
  assert.ok(draw({ gone: true }).includes('もうボードにありません'));
  assert.ok(aidePartHtml({ id: 'p1', kind: 'request', ref }, true, {}).includes('読み込み中'));
});

test('the tag a gate is named by is typed as text the aide reads, cleaned of anything that would be a key', () => {
  assert.equal(aideGateTagText('acme-widget-abc/g1'), '[gate acme-widget-abc/g1] ');
  assert.equal(aideGateTagText('a/b\nc'), '[gate a/b c] ');
});

test('a request that is done with no PR ends in 完了, and no step is left at now', () => {
  const steps = stepsOf(requestTask({ needsReading: false, issue: '#42', status: 'done', worktree: '/w' }));
  assert.equal(stateOf(steps, 'pr'), 'todo');
  assert.equal(steps.at(-1).key, 'done');
  assert.equal(steps.at(-1).state, 'done');
  assert.ok(steps.at(-1).text.includes('完了'));
  assert.ok(!steps.some(s => s.state === 'now'));
  // With a PR it is the PR step that is done, and there is no extra line.
  const withPr = stepsOf(requestTask({ needsReading: false, issue: '#42', status: 'done', worktree: '/w', pr: 'https://github.com/acme/widget/pull/9' }));
  assert.equal(stateOf(withPr, 'pr'), 'done');
  assert.ok(!withPr.some(s => s.key === 'done'));
});

test('aideRequestSteps: a worktree kept on a queued task is not a started worker', () => {
  const steps = aideRequestSteps({ needsReading: false, issue: '#42', status: 'queued', worktree: '/w' }, []);
  assert.equal(steps.find(s => s.key === 'worker').state, 'now');
});
