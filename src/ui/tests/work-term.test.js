/* The pure half of what is laid over the middle terminal (src/ui/work-term.js), run with `node --test`. Loaded the way the browser
   does, into one context; what needs the page is cut out of decide.js and session-actions.js. */
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

const ctx = vm.createContext({ TextEncoder, TextDecoder });
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'gate-evidence.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(cut(read('session-actions.js'), /^const actionButtonHtml = [^\n]*\n[^\n]*\n[^\n]*\n/m), 'session-actions.js');
run(read('question-card.js'), 'question-card.js');
// my-work-clean.js is loaded whole (it only defines); the two page helpers it asks of my-work.js and task-view.js are stood in for.
run('const workMerged = r => (r.task ? r.task.prState === "merged" : r.s?.branchPr?.state === "merged"); const prRefNumber = pr => /(\\d+)$/.exec(pr || "")?.[1] || "";', 'page.js');
run(read('my-work-clean.js'), 'my-work-clean.js');
run(read('work-term.js'), 'work-term.js');
const api = vm.runInContext(
  '({ workSendableText, workSendBody, workSendTooLong, workTermGatesOf, workTermAskHtml, workTermMergedHtml, workTermHeadActsHtml, workTermMoreHtml, workHubStatusLineHtml, workHubStoppedHtml, workStoppedHubShown, workTermFabsHtml, workSendParts })', ctx);

const keyOf = g => `s/${g.id}`;
const ctxOf = g => ({ task: null, nwo: 'acme/widget', key: keyOf(g), issue: { number: '', url: '', hasBody: false } });
const dispatch = { id: 'd1', kind: 'dispatch', title: 'Start?', focus: 'Start on **this**?', wait: true, options: ['approve', 'changes'], openedAt: '20260922T041233Z' };
const quick = { id: 'q1', kind: 'question', quick: true, focus: 'Which one?', wait: true, choices: [{ id: 'a', label: 'Alpha' }, { id: 'b', label: 'Beta' }] };
const heavy = { id: 'h1', kind: 'plan', focus: 'Is the plan fine?', title: 'Plan', wait: true, options: ['approve'] };
const count = (h, needle) => h.split(needle).length - 1;

test('the start check of a hub is a form with a fold button, and folded it is one line with the gate\'s first button', () => {
  const open = api.workTermAskHtml([dispatch], { ctxOf, keyOf, hub: true });
  assert.ok(open.includes('wk-ask-form'));
  assert.ok(open.includes('gate-comment'));
  assert.ok(open.includes('data-wk-fold="s/d1"') && open.includes('aria-expanded="true"'));
  const folded = api.workTermAskHtml([dispatch], { ctxOf, keyOf, hub: true, folded: new Set(['s/d1']) });
  assert.ok(!folded.includes('gate-comment') && !folded.includes('wk-ask-form'));
  assert.ok(folded.includes('data-qc-gate="s/d1"') && folded.includes('aria-expanded="false"'));
  assert.equal(count(folded, 'data-act="approve"'), 1);
  assert.ok(!folded.includes('data-act="changes"'));
});

test('a folded start check with a comment typed leaves the answer button out: it would send without the comment', () => {
  const folded = api.workTermAskHtml([dispatch], { ctxOf, keyOf, hub: true, folded: new Set(['s/d1']), draftOf: () => 'wait' });
  assert.ok(!folded.includes('data-act='));
  assert.ok(folded.includes('data-wk-fold="s/d1"'));
});

test('a dispatch question is no form for a task\'s terminal, and a quick one is never the form', () => {
  const task = api.workTermAskHtml([dispatch], { ctxOf, keyOf, hub: false });
  assert.ok(!task.includes('wk-ask-form') && task.includes('data-qc-open-gate="s/d1"'));
  const q = api.workTermAskHtml([{ ...dispatch, quick: true, choices: [{ id: 'a', label: 'Go' }] }], { ctxOf, keyOf, hub: true });
  assert.ok(!q.includes('wk-ask-form') && q.includes('qc-quick'));
});

test('a quick gate is a row answered in place, any other a line that opens the card', () => {
  const html = api.workTermAskHtml([quick, heavy], { ctxOf, keyOf });
  assert.ok(html.includes('data-choice="a"') && html.includes('data-choice="b"'));
  assert.ok(html.includes('data-qc-open-gate="s/h1"') && html.includes('開いて答える'));
  assert.ok(!html.includes('gate-comment'));
});

test('a gate of the page is not listed for a task of another board, nor one that was answered, nor one that no longer waits', () => {
  const task = { id: 't1', _slug: 'b1' };
  const gates = [
    { id: 'a', task: 't1', _slug: 'b1', wait: true }, { id: 'b', task: 't1', _slug: 'b2', wait: true },
    { id: 'c', task: 't1', _slug: 'b1', wait: false }, { id: 'd', task: 't1', _slug: 'b1', wait: true }, { id: 'e', task: 't2', _slug: 'b1', wait: true },
  ];
  const got = api.workTermGatesOf({ task, gates, taskOfGate: () => null, answered: new Set(['s/d']), keyOf });
  assert.deepEqual(got.map(g => g.id), ['a']);
});

test('a hub lists what it waits on that no task card shows', () => {
  const sessGates = [{ id: 'x', wait: true }, { id: 'y', wait: true, task: 't1' }, { id: 'z', wait: true }];
  const got = api.workTermGatesOf({ sessGates, gates: [], taskOfGate: g => (g.task ? { id: g.task } : undefined), answered: new Set(['s/z']), keyOf });
  assert.deepEqual(got.map(g => g.id), ['x']);
});

test('a hub also lists its dispatch gates (the start check) that a task names, but a task\'s other gates stay on the card', () => {
  const sessGates = [{ id: 's', kind: 'dispatch', wait: true, task: 't1' }, { id: 'p', kind: 'plan', wait: true, task: 't1' }];
  const withTask = () => ({ id: 't1' });
  const base = { sessGates, gates: [], taskOfGate: withTask, answered: new Set(), keyOf };
  assert.deepEqual(api.workTermGatesOf({ ...base, hub: true }).map(g => g.id), ['s']);
  assert.deepEqual(api.workTermGatesOf({ ...base }).map(g => g.id), []);
});

test('what a title or a question says is escaped', () => {
  const html = api.workTermAskHtml([{ ...heavy, focus: '<img src=x onerror=1>', title: '<script>' }], { ctxOf, keyOf });
  assert.ok(!html.includes('<img') && !html.includes('<script>'));
});

test('the heading buttons are offered only where they apply', () => {
  const none = api.workTermHeadActsHtml({});
  assert.equal(none, '');
  const task = api.workTermHeadActsHtml({ task: true, ide: true, close: true });
  assert.ok(task.includes('data-wk-term-act="detail"') && task.includes('タスク詳細'));
  assert.ok(task.includes('data-wk-term-act="ide"') && task.includes('data-wk-term-act="close"') && task.includes('セッションを閉じる'));
  assert.ok(!task.includes('hubinfo') && !task.includes(' disabled'));
  // 「下読みを見る」 / 「下読みする」 comes before 「タスク詳細」, for a task only.
  const see = api.workTermHeadActsHtml({ task: true, ownRead: 'see' });
  assert.match(see, /^<button type="button" class="btn-m3-text" data-own-read="see"[^>]*>.*auto_stories.*下読みを見る/);
  assert.ok(see.indexOf('data-own-read') < see.indexOf('タスク詳細'));
  const ask = api.workTermHeadActsHtml({ task: true, ownRead: 'ask' });
  assert.ok(ask.includes('data-own-read="ask"') && ask.includes('下読みする') && !ask.includes('下読みを見る'));
  assert.ok(!api.workTermHeadActsHtml({ task: true }).includes('data-own-read'));
  assert.ok(!api.workTermHeadActsHtml({ ownRead: 'see' }).includes('data-own-read'));
  const noTask = api.workTermHeadActsHtml({ ide: true });
  assert.ok(!noTask.includes('タスク詳細') && !noTask.includes('close'));
  const hub = api.workTermHeadActsHtml({ hub: true });
  assert.ok(hub.includes('data-wk-term-act="hubinfo"') && hub.includes('hub の詳細') && !hub.includes('タスク詳細'));
});

test('a close that cannot be done now is disabled, with the reason', () => {
  const html = api.workTermHeadActsHtml({ close: true, closeWhy: '再起動しています' });
  assert.ok(/data-wk-term-act="close" disabled title="再起動しています"/.test(html));
});

test('a hub\'s status line escapes the words it is given', () => {
  const html = api.workHubStatusLineHtml('稼働中', 'pill-good', [['受信箱', '3 件'], ['<b>', '1']]);
  assert.ok(html.includes('m3-pill pill-good') && html.includes('受信箱 3 件'));
  assert.ok(!html.includes('<b>'));
});

test('the block over a stopped hub says how long, what waits, and carries the start button with its reason', () => {
  const act = { act: 'hub-start', icon: 'play_arrow', label: 'hub を起動', title: 'tmux が使えません', disabled: true };
  const html = api.workHubStoppedHtml({ since: '3 分前', unread: 2, act });
  assert.ok(html.includes('hub は3 分前から止まっています'));
  assert.ok(html.includes('受信箱に未読の依頼が 2 件'));
  assert.ok(html.includes('data-sess-act="hub-start" title="tmux が使えません" disabled'));
  const quiet = api.workHubStoppedHtml({ act: { ...act, disabled: false, title: '' } });
  assert.ok(quiet.includes('hub は止まっています') && !quiet.includes('受信箱') && !quiet.includes('disabled'));
});

test('the floating buttons: the last message only when there is one, and sending disabled with its reason', () => {
  const both = api.workTermFabsHtml({ hasLast: true, lastAge: '2 分前' });
  assert.ok(both.includes('data-wk-fab="lastmsg"') && both.includes('2 分前') && both.includes('data-wk-fab="send"') && !both.includes('disabled'));
  const only = api.workTermFabsHtml({ sendWhy: 'ターミナルは切れています' });
  assert.ok(!only.includes('lastmsg') && only.includes('disabled title="ターミナルは切れています"'));
});

test('the send box carries the draft escaped, the Enter choice, and both buttons', () => {
  const { body, foot } = api.workSendParts({ draft: '</textarea><b>x', enter: false });
  assert.ok(body.includes('&lt;/textarea&gt;&lt;b&gt;x') && body.includes('aria-label="ターミナルに送る内容"'));
  assert.ok(foot.includes('送ったら Enter も押す') && !foot.includes('checked'));
  assert.ok(foot.includes('data-wk-send="paste"') && foot.includes('data-wk-send="send"') && foot.includes('⌘ Enter'));
  assert.ok(api.workSendParts({}).foot.includes('checked'));
});

test('what is sent has the control characters out, and keeps the tab and the line break', () => {
  const t = api.workSendableText('a\tb\nc\x1b[201~d\x07e\r\nf\x7fg\u009bh');
  assert.equal(t, 'a\tb\nc[201~de\nfgh');
  assert.equal(api.workSendableText(null), '');
});

test('the stopped-hub block covers a hub with no process, also over a terminal that was mounted before it stopped', () => {
  const on = { isHub: true, present: false };
  assert.equal(api.workStoppedHubShown(on), true);
  assert.equal(api.workStoppedHubShown({ ...on, starting: true }), false);
  assert.equal(api.workStoppedHubShown({ ...on, present: true }), false);
  assert.equal(api.workStoppedHubShown({ present: false }), false);
});

test('what is sent has the line breaks around it dropped, and the ones inside kept', () => {
  assert.equal(api.workSendBody('\n\nfirst\nsecond\n\n'), 'first\nsecond');
  assert.equal(api.workSendBody('\n \n'), '');
  assert.equal(api.workSendBody(' \t\n  indented\nnext  \n \n'), '  indented\nnext  ');
});

test('a send over the cap is too long, counted in UTF-8 bytes', () => {
  assert.equal(api.workSendTooLong('a'.repeat(512 * 1024)), false);
  assert.equal(api.workSendTooLong('a'.repeat(512 * 1024 + 1)), true);
  assert.equal(api.workSendTooLong('あ'.repeat(180000)), true);
});

test('the start check of a stopped hub is drawn as its one line whatever was folded', () => {
  const html = api.workTermAskHtml([dispatch], { ctxOf, keyOf, hub: true, foldAll: true });
  assert.ok(html.includes('wk-ask-folded') && !html.includes('wk-ask-form') && !html.includes('data-wk-fold'));
});

test('a merged session\'s line over the terminal asks without naming the session, with both chips and both buttons', () => {
  const html = api.workTermMergedHtml({ ask: 'clean', title: 'Task <1>', number: '12', files: 0, unpushed: 0 }, { board: 'b1', sid: 's1' });
  assert.ok(html.includes('PR #12 マージ済み。片付けてセッションを閉じる？') && !html.includes('Task'));
  assert.ok(html.includes('未コミット 0') && html.includes('未 push 0'));
  assert.ok(html.includes('data-wk-ask-act="cleanup"') && html.includes('data-wk-ask-act="keep"') && html.includes('cleaning_services'));
  assert.ok(html.includes('data-wk-ask-board="b1"') && html.includes('data-wk-ask-sid="s1"') && !html.includes(' disabled'));
  assert.ok(api.workTermMergedHtml({ ask: 'clean', number: '' }, { board: 'b1', sid: 's1' }).includes('<b class="qc-quick-text">マージ済み。片付けて'));
  assert.ok(api.workTermMergedHtml({ ask: 'clean', number: '12' }, { board: 'b1', sid: 's1', busy: true }).includes(' disabled'));
});

test('the dirty line warns with the two counts, has no clean-up button and a button that looks in the terminal; the ids are escaped', () => {
  const html = api.workTermMergedHtml({ ask: 'dirty', title: 'T', number: '12', files: 2, unpushed: 3 }, { board: 'b<1', sid: 's"1' });
  assert.ok(html.includes('マージ済みですが、消えると困る変更があります（未 push 3 · 未コミット 2）'));
  assert.ok(html.includes('warning') && html.includes('aria-hidden="true"') && html.includes('>ターミナルで確かめる<') && html.includes('data-wk-ask-act="check"'));
  assert.ok(!html.includes('cleanup') && !html.includes('data-wk-ask-act="keep"') && !html.includes('qc-chip'));
  assert.ok(html.includes('data-wk-ask-board="b&lt;1"') && html.includes('data-wk-ask-sid="s&quot;1"'));
});

test('the heading of a parent has 「親タスクの詳細」 before 「hub の詳細」, and none otherwise', () => {
  const both = api.workTermHeadActsHtml({ parent: true, hub: true });
  assert.ok(both.includes('data-wk-term-act="parent"') && both.includes('account_tree') && both.includes('親タスクの詳細'));
  assert.ok(both.indexOf('親タスクの詳細') < both.indexOf('hub の詳細'));
  assert.ok(!api.workTermHeadActsHtml({ hub: true }).includes('親タスクの詳細'));
  assert.ok(!api.workTermHeadActsHtml({ task: true }).includes('data-wk-term-act="parent"'));
});

test('the ⋯ menu lists its items in the order given, disabled ones carry their titles, and no items give no button', () => {
  assert.equal(api.workTermMoreHtml([]), '');
  assert.equal(api.workTermMoreHtml(), '');
  const html = api.workTermMoreHtml([
    { act: 'link-new', icon: 'add_task', label: 'タスクにする…', disabled: true, title: 'セッションが動いていません' },
    { act: 'goto-board', icon: 'open_in_new', label: 'そのボードで開く', attrs: ' data-board="b&lt;1"' },
    { act: 'copy', icon: 'content_copy', label: 'パスをコピー <x>' },
  ]);
  assert.ok(html.includes('aria-label="ほかの操作"') && html.includes('aria-haspopup="menu"') && html.includes('aria-expanded="false"') && html.includes('more_horiz'));
  assert.ok(html.includes('role="menu"') && html.includes('hidden'));
  assert.equal(count(html, 'role="menuitem"'), 3);
  assert.ok(html.indexOf('data-wk-more-act="link-new"') < html.indexOf('data-wk-more-act="goto-board"') && html.indexOf('data-wk-more-act="goto-board"') < html.indexOf('data-wk-more-act="copy"'));
  const first = html.slice(html.indexOf('data-wk-more-act="link-new"'), html.indexOf('data-wk-more-act="goto-board"'));
  assert.ok(first.includes(' disabled') && first.includes('title="セッションが動いていません"'));
  const second = html.slice(html.indexOf('data-wk-more-act="goto-board"'), html.indexOf('data-wk-more-act="copy"'));
  assert.ok(!second.includes(' disabled') && second.includes('data-board="b&lt;1"'));
  assert.ok(html.includes('パスをコピー &lt;x&gt;') && !html.includes('<x>'));
});

test('a folded start check draws the lead answer button: the kind\'s primary, or the recommended choice, or none', () => {
  const fold = g => api.workTermAskHtml([g], { ctxOf, keyOf, hub: true, folded: new Set([keyOf(g)]) });
  assert.ok(fold(dispatch).includes('着手する') && fold(dispatch).includes('data-act="approve"'));
  const rec = fold({ ...dispatch, choices: [{ id: 'a', label: 'A. now' }, { id: 'b', label: 'B. later', recommended: true }] });
  assert.equal(count(rec, 'data-choice='), 1);
  assert.ok(rec.includes('data-choice="b"') && !rec.includes('data-act="approve"'));
  const none = fold({ ...dispatch, choices: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }] });
  assert.ok(!none.includes('data-choice=') && !none.includes('data-act=') && none.includes('data-wk-fold='));
});
