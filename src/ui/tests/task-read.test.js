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
const { taskReadHeadMode, ownReadSubText, ownReadTitleText, ownReadLineText, ownReadHeadLine, ownReadActsHtml, taskReadGroups, taskReadBandHtml, taskReadHubHtml,
  taskReaderHeadHtml } = ctx;

const T0 = Date.UTC(2026, 9, 1) / 1000;
const A = 'a'.repeat(40);
const B = 'b'.repeat(40);
const hub = { canRead: true, why: '' };
const rec = (extra = {}) => ({
  id: 'acme/widget#12', repo: 'acme/widget', number: 12, url: 'https://github.com/acme/widget/pull/12', title: 'Round the amounts',
  author: 'alice', base: 'main', head: 'feat', headSha: A, state: 'ai-ready', additions: 30, deletions: 4, changedFiles: 2,
  files: [{ path: 'src/a.rs', additions: 20, deletions: 3, change: 'modified' }, { path: 'src/b.rs', additions: 8, deletions: 1, change: 'modified' }],
  ci: { pass: 1, fail: 0, pending: 0 }, reviewers: [], events: { pushed: null },
  ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z', stale: false, counts: { attention: 2, check: 3, concerns: 1, drafts: 4 } }, ...extra,
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

test('the heading button of a task: 下読みを見る with a read-through or before the document has come, 下読みする without one, none for no PR or a merged one', () => {
  const mode = (t, d, prState = 'open') => taskReadHeadMode(t, d, T0, prState);
  assert.strictEqual(mode({ id: 't1' }, null), '');
  assert.strictEqual(mode({ id: 't1', pr: '' }, doc(rec())), '');
  // The PR's state is the page's (`prStateOf`), not a field of the board's task: merged or closed has no button.
  assert.strictEqual(mode(task, doc(rec()), 'merged'), '');
  assert.strictEqual(mode(task, doc(null), 'closed'), '');
  // A record that says the PR is done is never offered a read (the server refuses one); a read-through it has is still shown.
  assert.strictEqual(mode(task, doc(rec({ state: 'done', ai: null }))), '');
  assert.strictEqual(mode(task, doc(rec({ state: 'done' }))), 'see');
  // Before the document has come: see.
  assert.strictEqual(mode(task, null), 'see');
  // The task names a PR this cannot read: the dialog says so.
  assert.strictEqual(mode(task, { pr: null, record: null, hub: null }), 'see');
  // It has come: a read-through says see, none says ask (no record, no read, or a read that failed before there was one).
  assert.strictEqual(mode(task, doc(rec())), 'see');
  assert.strictEqual(mode(task, doc(null)), 'ask');
  assert.strictEqual(mode(task, doc(rec({ ai: null }))), 'ask');
  assert.strictEqual(mode(task, doc(rec({ ai: { status: 'failed', error: 'x' } }))), 'ask');
  // A read under way has nothing to ask for: see, which says it is under way.
  assert.strictEqual(mode(task, doc(rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' } }))), 'see');
  assert.strictEqual(mode(task, doc(rec({ ai: { status: 'running', launchedAt: '20261001T000000Z' } }))), 'see');
  // One that has hung is offered again, as the dialog offers it.
  assert.strictEqual(mode(task, doc(rec({ state: 'requested', ai: { status: 'running', launchedAt: '20260930T000000Z' } }))), 'ask');
  // An earlier push's read-through is still one.
  assert.strictEqual(mode(task, doc(rec({ headSha: B, ai: { status: 'ready', sha: A, stale: true } }))), 'see');
});

test('the dialog\'s head: the line over the title, the title, and the line under it', () => {
  assert.strictEqual(ownReadSubText(rec(), task), 'acme/widget · PR #12 · あなたのタスク');
  // Before there is a record: the repository and number the task\'s link names.
  assert.strictEqual(ownReadSubText(null, task), 'acme/widget · PR #12 · あなたのタスク');
  assert.strictEqual(ownReadTitleText(task), 'AI 下読み — Round the amounts');
  // A read-through: when the reader finished and what it counted, as the list words it.
  const done = ownReadLineText(rec(), T0 + 5 * 60 + 5);
  assert.strictEqual(done, '下読みの worker が5分前に読み終えました · 要注意 2 · 確認 3 · 気になる 1');
  assert.strictEqual(ownReadLineText(rec(), T0), '下読みの worker がたった今読み終えました · 要注意 2 · 確認 3 · 気になる 1');
  // Each count only when it is a number: a read-through of the newer form counts the concerns alone.
  assert.strictEqual(ownReadLineText(rec({ ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z', counts: { concerns: 2 } } }), T0), '下読みの worker がたった今読み終えました · 気になる 2');
  assert.strictEqual(ownReadLineText(rec({ ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z', counts: { attention: 0, check: 1, concerns: 'x' } } }), T0), '下読みの worker がたった今読み終えました · 要注意 0 · 確認 1');
  assert.strictEqual(ownReadLineText(rec({ ai: { status: 'ready', sha: A, finishedAt: '20261001T000000Z' } }), T0), '下読みの worker がたった今読み終えました');
  // Under way, or never read.
  assert.strictEqual(ownReadLineText(rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' } }), T0), '下読み中です');
  assert.strictEqual(ownReadLineText(open(), T0), '下読み中です');
  // A reader that has hung is not at work: the body offers the read again, so the line does not say it is under way.
  assert.strictEqual(ownReadLineText(open({ launchedAt: '20260930T000000Z' }), T0), 'まだ下読みしていません');
  assert.strictEqual(ownReadLineText(rec({ ai: null, state: 'requested' }), T0), 'まだ下読みしていません');
  assert.strictEqual(ownReadLineText(null, T0), 'まだ下読みしていません');
});

test('the dialog\'s line says nothing until the task\'s document has come, nor for a PR that cannot be read', () => {
  // Not come yet, or the first read failed: nothing is known, so nothing is said (the body says it is loading, or why it failed).
  assert.strictEqual(ownReadHeadLine(null, T0), '');
  // The task has no PR this can read: the body says so.
  assert.strictEqual(ownReadHeadLine({ pr: null, record: null }, T0), '');
  // A readable PR with no record yet was never read.
  assert.strictEqual(ownReadHeadLine({ pr: 'acme/widget#12', record: null }, T0), 'まだ下読みしていません');
  assert.strictEqual(ownReadHeadLine({ pr: 'acme/widget#12', record: rec() }, T0), ownReadLineText(rec(), T0));
});

test('the dialog\'s buttons: 読み直す while a read-through exists and the PR is live, 下読みを閉じる while a reader runs', () => {
  const acts = (r, read, h = hub, asking = false, closing = false) => ownReadActsHtml(r, read, h, asking, closing);
  const html = acts(rec(), readOf());
  assert.ok(/data-pr-reread>/.test(html) && html.includes('読み直す') && !html.includes('data-pr-close-reader'));
  // Disabled with the hub's reason, and 依頼中… while a request is out.
  assert.match(acts(rec(), readOf(), { canRead: false, why: 'hub が止まっています' }), /data-pr-reread disabled title="hub が止まっています"/);
  assert.ok(acts(rec(), readOf(), hub, true).includes('依頼中…'));
  // While the read-through is on its way it already exists.
  assert.ok(acts(rec(), undefined).includes('data-pr-reread'));
  // None before a read, none for a PR that waits for nothing.
  assert.strictEqual(acts(rec({ ai: null, state: 'requested' }), null), '');
  assert.ok(!acts(rec({ state: 'done' }), readOf()).includes('data-pr-reread'));
  assert.strictEqual(acts(null, null), '');
  // A running reader with a process can be closed.
  const running = acts(open({ status: 'running' }), null);
  assert.ok(running.includes('data-pr-close-reader') && running.includes('下読みを閉じる') && !running.includes('data-pr-reread'));
  assert.ok(acts(open({ status: 'running' }), null, hub, false, true).includes('閉じています…'));
  assert.ok(!acts(open({ pid: undefined }), null).includes('data-pr-close-reader'));
});

test('the groups are the dialog\'s without the comparison with your review, with the reader\'s card first', () => {
  const r = rec({ state: 'pushed', commitsSinceReview: 2 });
  const rd = readOf({ sinceReview: { comments: [], newChanges: [] } });
  assert.ok(plain(ctx.prDialogGroups(r, rd, [r], hub, T0)).some(g => g.key === 'since'), 'the dialog has it');
  const groups = plain(taskReadGroups(r, rd, hub, T0, {}));
  assert.ok(!groups.some(g => g.key === 'since'));
  assert.deepStrictEqual(groups.map(g => g.key), ['top', 'points', 'order', 'diff', 'about']);
  const top = groups[0].cards.map(c => c.key);
  assert.strictEqual(top[0], 'top:reader');
  assert.ok(top.includes('top:old') && top.includes('top:three'));
  assert.deepStrictEqual(groups[4].cards.map(c => c.key), ['top:facts', 'footer']);
  // The diff's files are in the reading order, and nothing can be opened to 「拡大」.
  assert.deepStrictEqual(groups[3].cards.map(c => c.key), ['diff:src/b.rs', 'diff:src/a.rs']);
  const everything = groups.map(g => g.cards.map(c => c.html).join('')).join('');
  assert.ok(!everything.includes('data-pr-expand') && !everything.includes('data-expand'));
  assert.ok(plain(groups).every(g => g.cards.every(c => c.expand === null)));
});

test('a review request\'s groups keep the comparison with your review and the queue\'s place, and a task\'s default does not', () => {
  const r = rec({ state: 'pushed', commitsSinceReview: 2 });
  const rd = readOf({ sinceReview: { comments: [], newChanges: [] } });
  const groups = plain(taskReadGroups(r, rd, hub, T0, { records: [r], since: true }));
  assert.deepStrictEqual(groups.map(g => g.key), ['since', 'top', 'points', 'order', 'diff', 'about']);
  assert.strictEqual(groups[1].cards[0].key, 'top:reader');
  // The default is the task's.
  assert.ok(!plain(taskReadGroups(r, rd, hub, T0, {})).some(g => g.key === 'since'));
  // The queue is the list's: a queued read of a review request says its place in it.
  const q = rec({ ai: { status: 'queued', queuedAt: '20261001T000000Z' } });
  const other = rec({ id: 'acme/widget#3', ai: { status: 'queued', queuedAt: '20260930T000000Z' } });
  const place = o => plain(taskReadGroups(q, null, hub, T0, o)).find(g => g.key === 'top').cards.map(c => c.html).join('');
  assert.notStrictEqual(place({ records: [q, other] }), place({}));
});

test('the reader\'s card has a 要点 group of its own when nothing else is there, after 前回のレビューから', () => {
  const bare = { version: 2, sha: A, madeAt: '20261001T000000Z', annotations: [], order: [], overall: null };
  const groups = plain(taskReadGroups(rec({ state: 'pushed', commitsSinceReview: 1 }), bare, hub, T0, { records: [], since: true }));
  assert.deepStrictEqual(groups.map(g => g.key).slice(0, 2), ['since', 'top']);
  assert.deepStrictEqual(groups[1].cards.map(c => c.key), ['top:reader']);
  assert.deepStrictEqual(plain(taskReadGroups(rec(), bare, hub, T0, {})).map(g => g.key).slice(0, 1), ['top']);
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

test('the groups of the dialog point at no terminal and offer no 「聞く」, while the middle\'s keep both', () => {
  const live = open({ status: 'ready' });
  const html = o => plain(taskReadGroups(live, readOf(), hub, T0, o)).map(g => g.cards.map(c => c.html).join('')).join('');
  const dialog = html({ terminal: false });
  assert.ok(!dialog.includes('右のターミナル') && !dialog.includes('data-pr-ask'));
  assert.ok(dialog.includes('data-pr-todraft') && dialog.includes('下読み済み'));
  assert.ok(html({}).includes('右のターミナル') && html({}).includes('data-pr-ask'));
  // Before a read: the card does not promise the terminal either.
  const r = rec({ ai: null, state: 'requested' });
  const before = plain(taskReadGroups(r, null, hub, T0, { terminal: false })).find(g => g.key === 'top').cards.map(c => c.html).join('');
  assert.ok(!before.includes('右のターミナル') && before.includes('data-pr-ai'));
});

test('before a read the middle is one group, and the card names where the reader will be', () => {
  const r = rec({ ai: null, state: 'requested' });
  const groups = plain(taskReadGroups(r, null, hub, T0, {}));
  assert.deepStrictEqual(groups.map(g => g.key), ['top', 'about']);
  assert.ok(groups[0].cards.find(c => c.key === 'top:ai').html.includes('読んだあとは、右のターミナルでその worker に差分のことを聞けます。'));
  assert.ok(groups[0].cards.find(c => c.key === 'top:ai').html.includes('data-pr-ai'));
});

test('without a record the dialog\'s line names the repository from the PR\'s ref, or the task\'s link, spelled as the work document has it', () => {
  assert.strictEqual(ownReadSubText(null, { pr: '855' }, { ref: 'acme/widget#855' }), 'acme/widget · PR #855 · あなたのタスク');
  assert.strictEqual(ownReadSubText(null, { pr: 'https://github.com/acme/widget/pull/855' }, {}), 'acme/widget · PR #855 · あなたのタスク');
  assert.strictEqual(ownReadSubText(null, { pr: '855' }, { ref: 'acme/widget#855', nwo: 'Acme/Widget' }), 'Acme/Widget · PR #855 · あなたのタスク');
  // Another repository in the work document is not the PR\'s.
  assert.strictEqual(ownReadSubText(null, { pr: '855' }, { ref: 'acme/widget#855', nwo: 'acme/other' }), 'acme/widget · PR #855 · あなたのタスク');
  assert.strictEqual(ownReadSubText(null, { pr: '855' }), 'PR #855 · あなたのタスク');
  assert.strictEqual(ownReadSubText(rec(), task, { ref: 'x/y#1' }), 'acme/widget · PR #12 · あなたのタスク');
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

test('the reader\'s head says what it is, and offers 「下読みを閉じる」 only while a reader runs that has a process', () => {
  const head = taskReaderHeadHtml(open(), false);
  assert.ok(head.includes('auto_stories') && head.includes('読み手') && head.includes('— 読むだけ · コードレビュー hub'));
  // The PR, the repository and the unregistered mark are in the middle's heading and the list.
  assert.ok(!head.includes('PR #12') && !head.includes('acme/widget') && !head.includes('href='));
  assert.ok(head.includes('data-pr-close-reader') && head.includes('下読みを閉じる') && head.includes('data-tp-close'));
  assert.ok(!head.includes('data-pr-expand'), 'no 「拡大」');
  assert.ok(taskReaderHeadHtml(open(), true).includes('閉じています…'));
  assert.ok(!taskReaderHeadHtml(open({ pid: undefined }), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(open({ readerClosedAt: '20261001T000009Z' }), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(rec(), false).includes('data-pr-close-reader'));
  assert.ok(!taskReaderHeadHtml(rec({ registered: false }), false).includes('未登録'));
});

/* ── what the buttons are on ── */

function resolver({ selected = 'pr:acme/widget#12', task: shownTask = null } = {}) {
  const w = { calls: [], asked: [], notes: [], edits: 0 };
  const c = vm.createContext({
    selectedTaskId: selected, PR_REF: 'pr:', CSS: { escape: x => x }, panelTerm: { name: 'panel' }, others: { doc: { hub: { state: 'running' } }, asking: new Set() },
    isPrRef: id => typeof id === 'string' && id.startsWith('pr:'), prIdOfRef: id => id.slice(3),
    othersById: id => (id === 'acme/widget#12' ? rec() : null), othersRecords: () => [rec()], othersHubView: () => hub,
    othersRequestAi: (id, mode) => w.asked.push(['list', id, mode]), renderTaskPanel: () => w.calls.push('panel'), taskReadRedraw: () => w.calls.push('read'),
    taskReadShown: () => shownTask, note: (...a) => w.notes.push(a), go: (p, o) => w.calls.push(['go', p, o]), nav: { pane: 'detail' },
    PR_HOOKS: { diff: null }, cardDialog: { subject: selected }, closeCardDialog: () => w.calls.push('close'), prDraftsEntry: () => ({ loaded: true }), prReadOf: () => readOf(), prDoc: () => ({ drafts: [] }),
    prEditDrafts: () => { w.edits++; return true; }, prRedrawNow() {}, prNextKey: () => 'p1', prDraftFromPoint: () => ({}), prDraftLocation: () => 'loc',
  });
  vm.runInContext(['prShown', 'prRecOf', 'prRedrawFor', 'prAsk', 'prAct'].map(n => fnSrc(panelSrc, n)).join('\n'), c);
  return { w, c, run: e => vm.runInContext(e, c) };
}
const onButton = sel => ({ target: { closest: s => (s.split(',').some(x => x.trim() === sel) ? { matches: m => m.split(',').some(x => x.trim() === sel), disabled: false, getAttribute: () => null, dataset: { prTodraft: '0', prAsk: '0' } } : null) } });

test('a review request is what the page is on while it is selected, a task\'s read-through while its dialog is open, and nothing otherwise', () => {
  const pr = resolver();
  const s = pr.run('prShown()');
  assert.strictEqual(s.kind, 'pr');
  assert.strictEqual(s.id, 'acme/widget#12');
  assert.strictEqual(s.rec.id, 'acme/widget#12');
  assert.strictEqual(s.slot.name, 'panel', 'its reader is in the panel');
  s.ask('split');
  assert.deepStrictEqual(pr.w.asked, [['list', 'acme/widget#12', 'split']]);
  // A task: what the task's own view says.
  const shown = { id: 'acme/widget#12', rec: rec({ title: 'the task\'s' }), kind: 'task', hubView: hub, asking: false, ask() {}, slot: null };
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

test('the buttons of a place belong to the thing the page is on: a task\'s read buttons ask through the task, a review request\'s through the list', () => {
  const asked = [];
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask: mode => asked.push(mode), slot: {} };
  const t = resolver({ selected: 't1', task: shown });
  t.c.e = onButton('[data-pr-split]');
  assert.strictEqual(t.run("prAct(e, { place: 'read' })"), true);
  t.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(t.run("prAct(e, { place: 'read' })"), true);
  assert.deepStrictEqual(asked, ['split', undefined]);
  assert.deepStrictEqual(t.w.asked, [], 'not through the list');
  // A click in a dialog that is not about what the page is on is nobody's.
  t.run("cardDialog.subject = 'pr:acme/widget#99'");
  assert.strictEqual(t.run("prAct(e, { place: 'dialog' })"), false);
  // A review request's middle is the same place 'read', and asks through the list.
  const pr = resolver();
  pr.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(pr.run("prAct(e, { place: 'read' })"), true);
  assert.strictEqual(pr.run('prAct(e)'), true);
  assert.deepStrictEqual(pr.w.asked, [['list', 'acme/widget#12', undefined], ['list', 'acme/widget#12', undefined]]);
  // A task whose PR has no record has no read to act on.
  const bare = resolver({ selected: 't1', task: { ...shown, rec: null } });
  bare.c.e = onButton('[data-pr-reread]');
  assert.strictEqual(bare.run("prAct(e, { place: 'read' })"), false);
});

test('a task\'s read-through takes drafts in its dialog as a review request does, with no tab to switch', () => {
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask() {}, slot: null };
  const t = resolver({ selected: 't1', task: shown });
  t.run("cardDialog.subject = 't1'");
  t.c.e = onButton('[data-pr-todraft]');
  assert.strictEqual(t.run("prAct(e, { place: 'dialog' })"), true);
  assert.strictEqual(t.w.edits, 1);
  assert.deepStrictEqual(t.w.calls.filter(x => Array.isArray(x)), []);
  // A review request's, in the middle, goes to 下書き.
  const pr = resolver();
  pr.c.e = onButton('[data-pr-todraft]');
  pr.run("prAct(e, { place: 'read' })");
  assert.strictEqual(pr.w.edits, 1);
  assert.deepStrictEqual(plain(pr.w.calls.filter(x => Array.isArray(x)).map(x => x[1])), [{ pane: 'drafts' }]);
});

test('「聞く」 reaches the reader\'s terminal of a review request, and a task\'s dialog has none to say when there is none', () => {
  const typed = [];
  const point = { path: 'src/a.rs', line: 4, title: 'T' };
  const pr = resolver();
  pr.c.panelTerm.term = { type: t2 => { typed.push(`pr:${t2}`); return true; }, focus() { typed.push('focus'); } };
  pr.c.panelTerm.taskId = 'pr:acme/widget#12/s1';
  pr.c.panelTerm.ended = null;
  pr.c.point = point;
  pr.run('prAsk(point)');
  assert.deepStrictEqual(typed, ['pr:loc の「T」について: ', 'focus']);
  // A reader that has ended is typed into by nobody.
  pr.c.panelTerm.ended = 4404;
  pr.run('prAsk(point)');
  assert.strictEqual(pr.w.notes.length, 1);
  assert.strictEqual(typed.length, 2);
  // A task\'s read-through has no terminal in its dialog: it says so and types nothing.
  const shown = { id: 'acme/widget#12', rec: rec(), kind: 'task', hubView: hub, asking: false, ask() {}, slot: null };
  const t = resolver({ selected: 't1', task: shown });
  t.c.point = point;
  t.run('prAsk(point)');
  assert.strictEqual(t.w.notes.length, 1);
  assert.deepStrictEqual(t.w.calls.filter(x => Array.isArray(x)), []);
});

test('「聞く」 in 「拡大」 closes the dialog first, and in the middle it does not', () => {
  const pr = resolver();
  pr.c.e = onButton('[data-pr-ask]');
  assert.strictEqual(pr.run("prAct(e, { place: 'dialog' })"), true);
  assert.strictEqual(pr.run('cardDialog.quiet'), true);
  assert.deepStrictEqual(pr.w.calls.filter(x => x === 'close'), ['close']);
  const panel = resolver();
  panel.c.e = onButton('[data-pr-ask]');
  assert.strictEqual(panel.run("prAct(e, { place: 'read' })"), true);
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

test('the dialog\'s clicks reach prAct as the review dialog\'s, the task\'s own dialog has a click of its own, and the other events follow both', () => {
  const dialogSrc = read('pr-dialog.js');
  const handlers = {};
  const calls = [];
  const c = vm.createContext({
    cardDialog: { mode: 'review' }, cardDialogDockEl: () => ({}), cardDialogSync() {}, prInput: e => calls.push(['input', e]), prFocusOut() {},
    prAct: (e, opts) => calls.push(['act', e, opts]), ownReadDialogClick: e => calls.push(['own', e]),
    cardDialogOnPr: () => ['review', 'own-read'].includes(c.cardDialog.mode),
    cardDialogEl: () => ({ addEventListener: (type, fn) => { handlers[type] = fn; } }),
  });
  vm.runInContext(['click', 'input'].map(t => cut(dialogSrc, new RegExp(`^cardDialogEl\\(\\)\\.addEventListener\\('${t}', e => \\{[\\s\\S]*?\\n\\}\\);`, 'm'))).join('\n'), c);
  handlers.click('E');
  handlers.input('I');
  assert.deepStrictEqual(plain(calls), [['act', 'E', { place: 'dialog' }], ['input', 'I']]);
  c.cardDialog.mode = 'own-read';
  handlers.click('F');
  handlers.input('J');
  assert.deepStrictEqual(plain(calls.slice(2)), [['own', 'F'], ['input', 'J']]);
  c.cardDialog.mode = 'task';
  handlers.click('G');
  handlers.input('K');
  assert.strictEqual(calls.length, 4, 'not for the task\'s dialog');
});

test('the task\'s dialog asks for a read from its own buttons, before the PR has a record too', () => {
  const calls = [];
  const btn = (sel, attrs = {}) => ({ closest: q => q.split(', ').includes(sel) ? { disabled: false, getAttribute: a => attrs[a] ?? null, matches: m => m === sel } : null });
  const c = vm.createContext({
    taskReadAsk: mode => calls.push(['ask', mode ?? null]), prAct: () => calls.push(['act']), prCloseReader: () => calls.push(['close']),
    othersStartHub: () => calls.push(['start']), OWN_HOOKS: { drafts: null },
  });
  vm.runInContext(fnSrc(read('task-read-view.js'), 'ownReadDialogClick'), c);
  c.ownReadDialogClick({ target: btn('[data-pr-ai]') });
  c.ownReadDialogClick({ target: btn('[data-pr-split]') });
  c.ownReadDialogClick({ target: btn('[data-pr-reread]', { 'aria-disabled': 'true' }) });
  c.ownReadDialogClick({ target: btn('[data-pr-close-reader]') });
  assert.deepStrictEqual(plain(calls), [['ask', null], ['ask', 'split'], ['close']]);
});

test('a queued read says 順番待ち without a place: the task\'s document holds its own record, not the queue', () => {
  const queued = rec({ ai: { status: 'queued', startedAt: '20261001T000000Z' } });
  const html = plain(taskReadGroups(queued, null, hub, T0, {})).map(g => g.cards.map(c => c.html).join('')).join('');
  assert.ok(html.includes('AI 下読みの順番待ち'), html);
  assert.ok(!html.includes('番目'));
});

test('a reader\'s terminal is not mounted into a pane that is hidden, and one already mounted is kept', () => {
  const mounts = [];
  const disposed = [];
  const c = vm.createContext({
    mountSessionTerminal: (host, o) => { mounts.push(o.path); return { name: 'term' }; }, disposeTermSlot: slot => { disposed.push(slot.taskId); Object.assign(slot, { taskId: null, term: null }); },
  });
  vm.runInContext(['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js'].map(read).join('\n'), c);
  vm.runInContext(fnSrc(panelSrc, 'syncReaderTerm'), c);
  const slot = { host: () => ({}), taskId: null, term: null, ended: null, reconnect: false, redraw() {} };
  c.slot = slot; c.rec = open({ status: 'ready' });
  vm.runInContext("syncReaderTerm(slot, rec, 'pr:x/s1', false)", c);
  assert.deepStrictEqual(mounts, [], 'hidden: nothing mounted');
  assert.strictEqual(slot.taskId, 'pr:x/s1');
  vm.runInContext("syncReaderTerm(slot, rec, 'pr:x/s1', true)", c);
  assert.strictEqual(mounts.length, 1);
  const before = disposed.length;
  vm.runInContext("syncReaderTerm(slot, rec, 'pr:x/s1', false)", c);
  assert.ok(slot.term && disposed.length === before, 'kept while hidden');
});

test('the tab says the code-review hub is stopped, and offers to start it only where the page can', () => {
  assert.equal(taskReadHubHtml(rec(), { state: 'running', canRead: true, why: '' }), '');
  assert.equal(taskReadHubHtml(null, undefined), '');
  const startable = { state: 'stopped', canRead: false, startable: true, why: 'x' };
  const html = taskReadHubHtml(rec(), startable);
  assert.match(html, /コードレビュー hub が止まっています/);
  assert.match(html, /<button type="button" class="btn-m3-tonal" data-task-read-hub-start>/);
  assert.match(html, /hub を起動/);
  assert.doesNotMatch(html, /aria-disabled/);
  const starting = taskReadHubHtml(rec(), startable, true);
  assert.match(starting, /data-task-read-hub-start aria-disabled="true"/);
  assert.match(starting, /起動中…/);
  // Before there is a record, the same.
  assert.match(taskReadHubHtml(null, startable), /data-task-read-hub-start/);
  const cannot = taskReadHubHtml(rec(), { state: 'stopped', canRead: false, startable: false, why: 'x' });
  assert.match(cannot, /adj hub --review/);
  assert.match(cannot, /tmux/);
  assert.doesNotMatch(cannot, /<button/);
});

test('the hub note is not offered for a PR that is done, and what a running hub lacks is escaped', () => {
  assert.equal(taskReadHubHtml(rec({ state: 'merged' }), { state: 'stopped', canRead: false, startable: true }), '');
  const html = taskReadHubHtml(rec(), { state: 'running', canRead: false, why: '<b>tmux</b> が要ります' });
  assert.match(html, /&lt;b&gt;tmux&lt;\/b&gt; が要ります/);
  assert.doesNotMatch(html, /<b>|<button/);
});
