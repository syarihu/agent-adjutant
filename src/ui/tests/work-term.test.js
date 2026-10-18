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
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'aide-display.js']) run(read(file), file);
const decide = read('decide.js');
run([
  cut(decide, /^const gateRef = [^\n]*\n/m),
  'const expandAttrs = () => ""; const expandBtnHtml = () => "";',
  cut(decide, /^const BUTTONS = \{[\s\S]*?\n\};/m),
  cut(decide, /^function choicesHtml\(g, pickable, opts = \{\}\) \{[\s\S]*?\n\}/m),
].join('\n'), 'decide.js');
run(cut(read('session-actions.js'), /^const actionButtonHtml = [^\n]*\n[^\n]*\n[^\n]*\n/m), 'session-actions.js');
run(read('question-card.js'), 'question-card.js');
run(read('work-term.js'), 'work-term.js');
const api = vm.runInContext(
  '({ workSendableText, workTermGatesOf, workTermAskHtml, workTermHeadActsHtml, workHubStatusLineHtml, workHubStoppedHtml, workTermFabsHtml, workSendParts })', ctx);

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
