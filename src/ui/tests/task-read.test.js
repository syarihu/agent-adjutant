/* A task's read-through as markup (src/ui/task-read.js) and the resolver that lets one set of buttons serve it and a review request
   (`prShown` in src/ui/pr-panel.js), run with `node --test`. The page's scripts share one global scope, so they are loaded into
   one context the way the browser does; what belongs to the task panel and the task view is stood in for. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const panelSrc = read('pr-panel.js');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const fnSrc = (src, name) => cut(src, new RegExp(`^(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`, 'm'));

const ctx = vm.createContext({
  secTitle: text => `<div class="tp-sec-title">${text}</div>`,
  panelBtnsHtml: jump => `<div class="tp-head-btns">${jump}<button data-tp-close></button></div>`,
  prRefNumber: pr => /(\d+)$/.exec(pr || '')?.[1] || '',
});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'task-read.js']) {
  vm.runInContext(read(file), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const { taskReadTabState, taskReadTabsHtml, taskReadHeadHtml, taskReadGroups, taskReadNoRecordHtml, taskReadBandHtml, taskReaderHeadHtml,
  taskReaderTabsHtml, taskReadDraftsPlaceholderHtml } = ctx;

const T0 = Date.UTC(2026, 9, 1) / 1000;
const A = 'a'.repeat(40);
const B = 'b'.repeat(40);
const hub = { canRead: true, why: '' };
const rec = (extra = {}) => ({
  id: 'acme/widget#12', repo: 'acme/widget', number: 12, url: 'https://github.com/acme/widget/pull/12', title: 'Round the amounts',
  author: 'alice', base: 'main', head: 'feat', headSha: A, state: 'ai-ready', additions: 30, deletions: 4, changedFiles: 2,
  files: [{ path: 'src/a.rs', additions: 20, deletions: 3, change: 'modified' }, { path: 'src/b.rs', additions: 8, deletions: 1, change: 'modified' }],
  ci: { pass: 1, fail: 0, pending: 0 }, reviewers: [], events: { pushed: null },
  ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z', stale: false, counts: { attention: 2, check: 3, drafts: 4 } }, ...extra,
});
const readOf = (extra = {}) => ({
  sha: A, madeAt: '20261001T000000Z', three: { purpose: 'P', approach: 'Ap', impact: 'Im', essence: 'Es' },
  points: [{ level: 'attention', path: 'src/a.rs', line: 4, title: 'T', why: 'W' }],
  order: [{ path: 'src/b.rs', category: 'logic', note: '', skippable: false }, { path: 'src/a.rs', category: 'test', note: '', skippable: false }],
  deleted: [], decisions: [], tests: { covered: [], notCovered: [] }, unwritten: [], drafts: [], overall: null, ...extra,
});
const task = { id: 't1', pr: 'https://github.com/acme/widget/pull/12', title: 'Round the amounts' };
const doc = (record, extra = {}) => ({ pr: 'acme/widget#12', record, hub: { state: 'running', max: 2, running: 0, queued: 0 }, ...extra });
const open = (extra = {}) => rec({ ai: { status: 'running', sha: null, launchedAt: '20261001T000000Z', sessionId: 's1', pid: 4242, terminal: { backend: 'tmux', window: '@3' }, ...extra } });

test('the tab says why it cannot be chosen, and otherwise the state of the read beside its label', () => {
  const state = (t, d, now = T0) => plain(taskReadTabState(t, d, now));
  assert.deepStrictEqual(state({ id: 't1' }, null), { disabled: true, why: 'PR ができたあとに使えます', sub: 'PR ができたあとに使えます', dot: false });
  assert.deepStrictEqual(state({ id: 't1', pr: '' }, doc(null)).disabled, true);
  // The task names a PR this cannot read.
  assert.deepStrictEqual(state(task, { pr: null, record: null, hub: null }), { disabled: true, why: 'この PR は AI 下読みできません', sub: 'この PR は AI 下読みできません', dot: false });
  // Nothing is said before the document has come, or while the PR has no record, or no read.
  assert.deepStrictEqual(state(task, null), { disabled: false, why: '', sub: '', dot: false });
  assert.deepStrictEqual(state(task, doc(null)), { disabled: false, why: '', sub: '', dot: false });
  assert.strictEqual(state(task, doc(rec({ ai: null }))).sub, '');
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' } }))).sub, '下読み中…');
  assert.strictEqual(state(task, doc(open())).sub, '下読み中…');
  // A reader that has hung is still reading as far as the tab goes.
  const hung = open({ launchedAt: '20260930T000000Z' });
  assert.strictEqual(state(task, doc(hung)).sub, '下読み中…');
  const ready = state(task, doc(rec()));
  assert.deepStrictEqual(ready, { disabled: false, why: '', sub: '要注意 2 · 確認 3', dot: false });
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'ready', sha: A, counts: { attention: 0, check: 1 } } }))).sub, '要注意 0 · 確認 1');
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'ready', sha: A } }))).sub, '');
  // The dot is for a read of an earlier push, and only then.
  const stale = state(task, doc(rec({ headSha: B, ai: { status: 'ready', sha: A, stale: true, counts: { attention: 2, check: 3 } } })));
  assert.deepStrictEqual(stale, { disabled: false, why: '', sub: '前の push 時点', dot: true });
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'failed', error: 'x' } }))).sub, '失敗しました');
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'failed', error: 'x' } }))).dot, false);
  assert.strictEqual(state(task, doc(rec({ ai: { status: 'failed', error: 'x' } }))).disabled, false);
});

test('the tabs: ids and what they control, the one that is on, a disabled one with its reason, and the dot', () => {
  const html = taskReadTabsHtml(plain(taskReadTabState({ id: 't1' }, null, T0)), 'task');
  assert.match(html, /id="wk-mid-tab-task"[^>]*data-mid-tab="task"[^>]*aria-selected="true"[^>]*aria-controls="wk-term-pane"/);
  assert.match(html, /id="wk-mid-tab-ai"[^>]*data-mid-tab="ai"[^>]*aria-selected="false"[^>]*aria-controls="task-read" disabled title="PR ができたあとに使えます"/);
  assert.ok(html.includes('role="tab"') && html.includes('PR ができたあとに使えます</span>'));
  assert.ok(!html.includes('tp-dot'));
  const on = taskReadTabsHtml(plain(taskReadTabState(task, doc(rec({ headSha: B, ai: { status: 'ready', sha: A, stale: true } })), T0)), 'ai');
  assert.match(on, /id="wk-mid-tab-ai"[^>]*aria-selected="true"/);
  assert.ok(on.includes('前の push 時点') && on.includes('class="tp-dot"'));
  assert.match(on, /id="wk-mid-tab-task"[^>]*aria-selected="false"/);
  // The tab that is on can always be left, so it is not disabled even where it could not be chosen.
  const stuck = taskReadTabsHtml({ disabled: true, why: 'x', sub: 'x', dot: false }, 'ai');
  assert.ok(!/id="wk-mid-tab-ai"[^>]* disabled/.test(stuck));
  // Everything is escaped.
  assert.ok(!taskReadTabsHtml({ disabled: false, why: '', sub: '<b>', dot: false }, 'task').includes('<b>'));
});

test('the head over the middle names the PR, and the task\'s title and repository', () => {
  const head = taskReadHeadHtml(task, rec());
  assert.ok(head.includes('AI 下読み — PR #12') && head.includes('Round the amounts · acme/widget'));
  // Before there is a record: the number the task names, and its own title.
  const bare = taskReadHeadHtml(task, null);
  assert.ok(bare.includes('AI 下読み — PR #12') && bare.includes('Round the amounts'));
  assert.ok(!taskReadHeadHtml({ id: 't', pr: '' }, null).includes('— PR'));
  assert.ok(!taskReadHeadHtml(task, rec({ title: '<img>' })).includes('<img>'));
});

test('the groups are the dialog\'s without the comparison with your review, with the reader\'s card first', () => {
  const r = rec({ state: 'pushed', commitsSinceReview: 2 });
  const rd = readOf({ sinceReview: { comments: [], newChanges: [] } });
  assert.ok(plain(ctx.prDialogGroups(r, rd, [r], hub, T0)).some(g => g.key === 'since'), 'the dialog has it');
  const groups = plain(taskReadGroups(r, rd, hub, T0, {}));
  assert.ok(!groups.some(g => g.key === 'since'));
  assert.deepStrictEqual(groups.map(g => g.key), ['top', 'points', 'order', 'diff']);
  const top = groups[0].cards.map(c => c.key);
  assert.strictEqual(top[0], 'top:reader');
  assert.ok(top.includes('top:facts') && top.includes('top:three') && top.includes('footer'));
  // The diff's files are in the reading order, and nothing can be opened to 「拡大」.
  assert.deepStrictEqual(groups[3].cards.map(c => c.key), ['diff:src/b.rs', 'diff:src/a.rs']);
  const everything = groups.map(g => g.cards.map(c => c.html).join('')).join('');
  assert.ok(!everything.includes('data-pr-expand') && !everything.includes('data-expand'));
  assert.ok(plain(groups).every(g => g.cards.every(c => c.expand === null)));
});

test('the reader\'s card says what the reader is: its state beside a read, and where it can be talked to', () => {
  const card = r => plain(taskReadGroups(r, readOf(), hub, T0, {})).find(g => g.key === 'top').cards.find(c => c.key === 'top:reader');
  const live = card(open({ status: 'ready' }));
  assert.ok(live.html.includes('下読みの worker は右のターミナルにいます'));
  assert.ok(live.html.includes('下読み済み'));
  const closed = card(rec({ ai: { ...rec().ai, readerClosedAt: '20261001T000009Z' } }));
  assert.ok(closed.html.includes('下読みの worker は閉じました'));
  // No window, not closed: only the state.
  const gone = card(rec());
  assert.ok(gone.html.includes('下読み済み') && !gone.html.includes('右のターミナル') && !gone.html.includes('閉じました'));
  // Before a read the card of the read itself says it, so this one has only the reader.
  const none = plain(taskReadGroups(open(), null, hub, T0, {})).find(g => g.key === 'top').cards;
  assert.ok(none.some(c => c.key === 'top:ai'));
  assert.ok(!(none.find(c => c.key === 'top:reader')?.html || '').includes('AI 下読み中'));
  // Nothing to say: no card.
  const quiet = plain(taskReadGroups(rec({ ai: null, state: 'requested' }), null, hub, T0, {})).find(g => g.key === 'top').cards;
  assert.ok(!quiet.some(c => c.key === 'top:reader'));
});

test('before a read the middle is one group, and the card names where the reader will be', () => {
  const r = rec({ ai: null, state: 'requested' });
  const groups = plain(taskReadGroups(r, null, hub, T0, {}));
  assert.deepStrictEqual(groups.map(g => g.key), ['top']);
  assert.ok(groups[0].cards.find(c => c.key === 'top:ai').html.includes('読んだあとは、右のターミナルでその worker に差分のことを聞けます。'));
  assert.ok(groups[0].cards.find(c => c.key === 'top:ai').html.includes('data-pr-ai'));
});

test('a task whose PR has no record offers 「AI 下読み」, disabled with the hub\'s reason, and says it is asked for', () => {
  const html = taskReadNoRecordHtml('acme/widget#12', hub, false);
  assert.ok(html.includes('まだ AI 下読みしていません') && html.includes('acme/widget#12') && /data-pr-ai>/.test(html));
  const off = taskReadNoRecordHtml('acme/widget#12', { canRead: false, why: 'hub が止まっています' }, false);
  assert.match(off, /data-pr-ai disabled title="hub が止まっています"/);
  assert.ok(taskReadNoRecordHtml('acme/widget#12', hub, true).includes('依頼中…'));
  assert.ok(!taskReadNoRecordHtml('<b>', hub, false).includes('<b>'));
});

test('the band says both commits and offers 「下読みし直す」, and only when the read is of an earlier push', () => {
  const stale = rec({ headSha: B, ai: { status: 'ready', sha: A, stale: true } });
  const band = taskReadBandHtml(stale, readOf(), hub, false);
  assert.ok(band.includes(`この差分は ${'a'.repeat(7)} で下読みしたときのものです（PR の head は ${'b'.repeat(7)}）`));
  assert.ok(/data-pr-reread>/.test(band) && band.includes('下読みし直す'));
  // Until the read has come, the record's own commit says it.
  assert.ok(taskReadBandHtml(stale, undefined, hub, false).includes(`${'a'.repeat(7)} で下読み`));
  assert.ok(taskReadBandHtml(stale, readOf(), hub, true).includes('依頼中…'));
  assert.match(taskReadBandHtml(stale, readOf(), { canRead: false, why: 'stopped' }, false), /data-pr-reread disabled title="stopped"/);
  assert.strictEqual(taskReadBandHtml(rec(), readOf(), hub, false), '');
  assert.strictEqual(taskReadBandHtml(null, null, hub, false), '');
  // A PR that is not waiting on anything cannot be read again: the line is there, the button is not.
  const done = taskReadBandHtml({ ...stale, state: 'done' }, readOf(), hub, false);
  assert.ok(done.includes('下読みしたときのものです') && !done.includes('data-pr-reread'));
});

test('the reader\'s head names the PR and the repository, and offers 「下読みを閉じる」 only while a reader runs that has a process', () => {
  const head = taskReaderHeadHtml(open(), false);
  assert.ok(head.includes('PR #12') && head.includes('acme/widget') && head.includes('読むだけ') && head.includes('下読みの worker'));
  assert.match(head, /href="https:\/\/github\.com\/acme\/widget\/pull\/12" target="_blank" rel="noopener noreferrer"/);
  assert.ok(head.includes('data-pr-close-reader') && head.includes('下読みを閉じる') && head.includes('data-tp-close'));
  assert.ok(!head.includes('data-pr-expand'), 'no 「拡大」');
  assert.ok(taskReaderHeadHtml(open(), true).includes('閉じています…'));
  assert.ok(!taskReaderHeadHtml(open({ pid: undefined }), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(open({ readerClosedAt: '20261001T000009Z' }), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(rec(), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(rec({ url: 'javascript:alert(1)' }), false).includes('javascript:'));
});

test('the reader\'s tabs are the terminal and 「worker への指示」 with its count, which needs a read-through', () => {
  const tabs = taskReaderTabsHtml('ai', 4, true);
  assert.match(tabs, /id="tp-tab-read-term"[^>]*data-read-pane="ai"[^>]*aria-selected="true"/);
  assert.match(tabs, /id="tp-tab-read-drafts"[^>]*data-read-pane="ai-drafts"[^>]*aria-selected="false"/);
  assert.match(tabs, /id="tp-tab-read-term"[^>]*aria-controls="tp-term"/);
  assert.match(tabs, /id="tp-tab-read-drafts"[^>]*aria-controls="tp-read-drafts"/);
  assert.ok(tabs.includes('ターミナル') && tabs.includes('worker への指示') && tabs.includes('>4</span>'));
  assert.ok(!/id="tp-tab-read-drafts"[^>]* disabled/.test(tabs));
  const second = taskReaderTabsHtml('ai-drafts', 0, true);
  assert.match(second, /id="tp-tab-read-drafts"[^>]*aria-selected="true"/);
  assert.ok(!second.includes('m3-tab-badge'));
  // Before a read: the second is off, with the reason the review request's tabs give.
  const before = taskReaderTabsHtml('ai', 0, false);
  assert.match(before, /id="tp-tab-read-drafts"[^>]* disabled title="AI 下読みのあとに使えます"/);
  assert.ok(!/id="tp-tab-read-term"[^>]* disabled/.test(before));
});

test('「worker への指示」 says how many drafts there are and that nothing is passed on yet, in the place a later change fills', () => {
  const html = taskReadDraftsPlaceholderHtml(3);
  assert.ok(html.includes('id="tp-read-drafts"') && html.includes('data-read-drafts'));
  assert.ok(html.includes('role="tabpanel"') && html.includes('aria-labelledby="tp-tab-read-drafts"'));
  assert.ok(html.includes('下読みの下書きが 3 件あります。worker に渡す操作は、まだありません。'));
  assert.ok(taskReadDraftsPlaceholderHtml(undefined).includes('0 件'));
});

/* ── what the buttons are on ── */

function resolver({ selected = 'pr:acme/widget#12', task: shownTask = null } = {}) {
  const w = { calls: [], asked: [], notes: [], edits: 0 };
  const c = vm.createContext({
    selectedTaskId: selected, PR_REF: 'pr:', CSS: { escape: x => x }, workTerm: { name: 'work' }, others: { doc: { hub: { state: 'running' } }, asking: new Set() },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3),
    othersById: id => (id === 'acme/widget#12' ? rec() : null), othersRecords: () => [rec()], othersHubView: () => hub,
    othersRequestAi: (id, mode) => w.asked.push(['list', id, mode]), renderTaskPanel: () => w.calls.push('panel'), taskReadRedraw: () => w.calls.push('read'),
    taskReadShown: () => shownTask, note: (...a) => w.notes.push(a), go: (p, o) => w.calls.push(['go', p, o]), nav: { pane: 'ai-drafts' },
    PR_HOOKS: { diff: null }, cardDialog: { subject: selected }, closeCardDialog: () => w.calls.push('close'), prDraftsEntry: () => ({ loaded: true }), prReadOf: () => readOf(), prDoc: () => ({ drafts: [] }),
    prEditDrafts: () => { w.edits++; return true; }, prRedrawNow() {}, prNextKey: () => 'p1', prDraftFromPoint: () => ({}), prDraftLocation: () => 'loc',
  });
  vm.runInContext(['prShown', 'prRecOf', 'prRedrawFor', 'prAsk', 'prAct'].map(n => fnSrc(panelSrc, n)).join('\n'), c);
  return { w, c, run: e => vm.runInContext(e, c) };
}
const onButton = sel => ({ target: { closest: s => (s.split(',').some(x => x.trim() === sel) ? { matches: m => m.split(',').some(x => x.trim() === sel), disabled: false, getAttribute: () => null, dataset: { prTodraft: '0', prAsk: '0' } } : null) } });

test('a review request is what the page is on while it is selected, a task\'s read-through while that tab is on, and nothing otherwise', () => {
  const pr = resolver();
  const s = pr.run('prShown()');
  assert.strictEqual(s.kind, 'pr');
  assert.strictEqual(s.id, 'acme/widget#12');
  assert.strictEqual(s.rec.id, 'acme/widget#12');
  assert.strictEqual(s.slot.name, 'work', 'its reader is in the middle');
  s.ask('split');
  assert.deepStrictEqual(pr.w.asked, [['list', 'acme/widget#12', 'split']]);
  // A task: what the task's own view says.
  const shown = { id: 'acme/widget#12', rec: rec({ title: 'the task\'s' }), kind: 'task', hubView: hub, asking: false, ask() {}, slot: { name: 'panel' } };
  const t = resolver({ selected: 't1', task: shown });
  assert.strictEqual(t.run('prShown()'), shown);
  assert.strictEqual(t.run('prRecOf("acme/widget#12")').title, 'the task\'s', 'its record, not the list\'s');
  assert.strictEqual(t.run('prRecOf("acme/widget#9")'), null, 'another PR is the list\'s to say');
  // Neither: nothing.
  assert.strictEqual(resolver({ selected: 't1' }).run('prShown()'), null);
  assert.strictEqual(resolver({ selected: null }).run('prShown()'), null);
});

test('a redraw for a PR draws what shows it: the panel for a review request, the task\'s view for a task, and nothing for another PR', () => {
  const pr = resolver();
  pr.run('prRedrawFor("acme/widget#12")');
  pr.run('prRedrawFor("acme/widget#9")');
  assert.deepStrictEqual(pr.w.calls, ['panel']);
  const t = resolver({ selected: 't1', task: { id: 'acme/widget#12', rec: rec(), kind: 'task' } });
  t.run('prRedrawFor("acme/widget#12")');
  t.run('prRedrawFor("acme/widget#9")');
  assert.deepStrictEqual(t.w.calls, ['read']);
});

test('the buttons of a place belong to the place the page is on: a task\'s read buttons ask through the task, and the panel\'s are not theirs', () => {
  const asked = [];
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask: mode => asked.push(mode), slot: {} };
  const t = resolver({ selected: 't1', task: shown });
  t.c.e = onButton('[data-pr-split]');
  assert.strictEqual(t.run("prAct(e, { place: 'read' })"), true);
  t.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(t.run("prAct(e, { place: 'read' })"), true);
  assert.deepStrictEqual(asked, ['split', undefined]);
  assert.deepStrictEqual(t.w.asked, [], 'not through the list');
  // The panel's and the dialog's clicks are for a review request only.
  assert.strictEqual(t.run('prAct(e)'), false);
  assert.strictEqual(t.run("prAct(e, { place: 'dialog' })"), false);
  // And the other way: a review request is not a place 'read'.
  const pr = resolver();
  pr.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(pr.run("prAct(e, { place: 'read' })"), false);
  assert.strictEqual(pr.run('prAct(e)'), true);
  assert.deepStrictEqual(pr.w.asked, [['list', 'acme/widget#12', undefined]]);
  // A task whose PR has no record has no read to act on.
  const bare = resolver({ selected: 't1', task: { ...shown, rec: null } });
  bare.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(bare.run("prAct(e, { place: 'read' })"), false);
});

test('a task\'s read-through has no drafts to add to: 「下書きに」 is taken and does nothing', () => {
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask() {}, slot: {} };
  const t = resolver({ selected: 't1', task: shown });
  t.c.e = onButton('[data-pr-todraft]');
  assert.strictEqual(t.run("prAct(e, { place: 'read' })"), true);
  assert.strictEqual(t.w.edits, 0);
  assert.deepStrictEqual(t.w.calls.filter(x => Array.isArray(x)), [], 'the panel did not go to 下書き');
  // Where drafts are offered the same click starts one.
  const pr = resolver();
  pr.c.e = onButton('[data-pr-todraft]');
  pr.run('prAct(e)');
  assert.strictEqual(pr.w.edits, 1);
});

test('「聞く」 of a task goes to the reader\'s terminal in the panel, showing it first when 「worker への指示」 is on, and says when there is none', () => {
  const typed = [];
  const slot = { term: { type: t => { typed.push(t); return true; }, focus() { typed.push('focus'); } }, ended: null, taskId: 'pr:acme/widget#12/s1' };
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask() {}, slot };
  const t = resolver({ selected: 't1', task: shown });
  t.c.point = { path: 'src/a.rs', line: 4, title: 'T' };
  t.run('prAsk(point)');
  assert.deepStrictEqual(plain(t.w.calls.filter(x => Array.isArray(x)).map(x => x[1])), [{ pane: 'ai' }], 'the terminal is shown first');
  assert.deepStrictEqual(typed, ['loc の「T」について: ', 'focus']);
  // Already on the terminal: nothing to switch.
  const on = resolver({ selected: 't1', task: shown });
  on.c.nav.pane = 'ai';
  on.c.point = t.c.point;
  on.run('prAsk(point)');
  assert.deepStrictEqual(on.w.calls, []);
  // A reader that has ended, or one that is not there.
  slot.ended = 4404;
  const ended = resolver({ selected: 't1', task: shown });
  ended.c.point = t.c.point;
  ended.run('prAsk(point)');
  assert.strictEqual(ended.w.notes.length, 1);
  assert.strictEqual(typed.length, 4, 'an ended reader is typed into by nobody');
  // The review request's reader is in the middle: its own slot.
  const pr = resolver();
  pr.c.workTerm.term = { type: t2 => { typed.push(`pr:${t2}`); return true; }, focus() {} };
  pr.c.workTerm.taskId = 'pr:acme/widget#12/s1';
  pr.c.workTerm.ended = null;
  pr.c.point = t.c.point;
  pr.run('prAsk(point)');
  assert.strictEqual(typed[typed.length - 1], 'pr:loc の「T」について: ');
});

test('「聞く」 in 「拡大」 closes the dialog first, and in the panel or the middle of a task it does not', () => {
  const pr = resolver();
  pr.c.e = onButton('[data-pr-ask]');
  assert.strictEqual(pr.run("prAct(e, { place: 'dialog' })"), true);
  assert.strictEqual(pr.run('cardDialog.quiet'), true);
  assert.deepStrictEqual(pr.w.calls.filter(x => x === 'close'), ['close']);
  const panel = resolver();
  panel.c.e = onButton('[data-pr-ask]');
  assert.strictEqual(panel.run('prAct(e)'), true);
  assert.deepStrictEqual(panel.w.calls.filter(x => x === 'close'), []);
  // A click in a dialog about another subject is nobody's here.
  const other = resolver();
  other.run("cardDialog.subject = 'pr:acme/widget#99'");
  other.c.e = onButton('[data-pr-ask]');
  assert.strictEqual(other.run("prAct(e, { place: 'dialog' })"), false);
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask() {}, slot: {} };
  const t = resolver({ selected: 't1', task: shown });
  t.c.e = onButton('[data-pr-ask]');
  t.run("prAct(e, { place: 'read' })");
  assert.deepStrictEqual(t.w.calls.filter(x => x === 'close'), []);
});

test('the dialog\'s clicks reach prAct as the dialog\'s, and the other events as before', () => {
  const dialogSrc = read('pr-dialog.js');
  const handlers = {};
  const calls = [];
  const c = vm.createContext({
    cardDialog: { mode: 'review' }, cardDialogDockEl: () => ({}), cardDialogSync() {}, prInput: e => calls.push(['input', e]), prFocusOut() {},
    prAct: (e, opts) => calls.push(['act', e, opts]),
    cardDialogEl: () => ({ addEventListener: (type, fn) => { handlers[type] = fn; } }),
  });
  vm.runInContext(['click', 'input'].map(t => cut(dialogSrc, new RegExp(`^cardDialogEl\\(\\)\\.addEventListener\\('${t}', e => \\{[\\s\\S]*?\\n\\}\\);`, 'm'))).join('\n'), c);
  handlers.click('E');
  assert.deepStrictEqual(plain(calls), [['act', 'E', { place: 'dialog' }]]);
  c.cardDialog.mode = 'task';
  handlers.click('F');
  assert.strictEqual(calls.length, 1, 'not for the task\'s dialog');
});
