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
  aideAskRowText, aideStateOf, aideNewLineHtml, aideHeadHtml, aideScopeText, aideCleanText, kindOf } = vm.runInContext(
  `({ aideEmptyDisplay, aideDisplayOf, aideDisplayWithWaits, registerAidePart, aideDisplayHtml, aidePartHtml, aideWaits, aidePinSub, aidePeekCount,
     aideAskRowText, aideStateOf, aideNewLineHtml, aideHeadHtml, aideScopeText, aideCleanText, kindOf })`, ctx);

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

test('a waits part with no renderer says it cannot be shown yet, and a null shownAt breaks nothing', () => {
  const html = aideDisplayHtml(aideDisplayWithWaits(aideEmptyDisplay(), SOME_WAITS));
  assert.ok(html.includes('この部品はまだ表示できません'));
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
  assert.ok(html.includes('この部品はまだ表示できません'));
  const nulled = aideDisplayHtml({ ...server, parts: server.parts.map(p => ({ ...p, shownAt: null })) });
  assert.ok(!nulled.includes('Invalid Date'));
});
