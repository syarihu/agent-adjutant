/* A long last message in the hub detail dialog (agentFactsHtml in src/ui/sessions-side.js, workHubInfoHtml in
   src/ui/work-term-view.js, #question-dialog .tp-lastmsg in src/ui/question-card.css), run with `node --test`. */
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

const ensured = [];
const detailCalls = [];
const ctx = vm.createContext({
  state: { now: 1000 }, minutesSince: (a, b) => (b - a) / 60, agoLabel: m => `${m}分前`,
  sessView: { lastMsg: null }, enc: encodeURIComponent,
  api: () => new Promise(() => {}), lastMessageArrived() {},
  // The page's helpers that the cut functions ask of other files.
  kv: (k, v) => `<div>${k}${v}</div>`, monoKv: (k, v) => `<div>${k}${v}</div>`,
  agentStateOf: () => null, ledgerState: () => null, sideNote: t => t, unknownWhy: () => '',
  STATE_LABEL: {}, permissionLabel: () => '',
  workHubWaitingHtml: () => '', hubTitleMarkHtml: () => '', hubTitleReadButtonHtml: () => '', hubTitleWhyHtml: () => '',
  hubPanelWakeBase: () => '',
  hubDetailHtml: (h, s, opts) => { detailCalls.push(opts); return '<p>before</p><button type="button" data-expand-open="lastmsg">x</button><p>after</p>'; },
});
vm.runInContext([
  read('util.js'),
  cut(read('card-dialog.js'), /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  cut(read('sessions-side.js'), /^const LAST_MESSAGE_SHOWN_CHARS = [^\n]*\n/m),
  cut(read('sessions-side.js'), /^const lastMessageKey = [^\n]*\n/m),
  cut(read('sessions-side.js'), /^const lastMessageCut = [\s\S]*?\n\};\n/m),
  cut(read('sessions-side.js'), /^function ensureLastMessage\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function lastMessageBodyHtml\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function lastMessageHtml\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function agentFactsHtml\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('work-term-view.js'), /^const workHubInfoHtml = [\s\S]*?\n\};/m),
].join('\n'), ctx);

const LONG_WORD = 'x'.repeat(2000);
const lines = Array.from({ length: 300 }, (_, i) => `line ${i} ${'word '.repeat(60)}`);
const WHOLE = `${LONG_WORD}\n\n${lines.join('\n\n')}\n\nEND`;
const CUT = `${WHOLE.slice(0, 1000)}…`;
const sess = () => ({ id: 'w1', present: true, agentSession: { sessionId: 'a1', lastMessage: CUT, lastMessageAt: 900, status: 'idle' } });

test('agentFactsHtml draws the whole long message when expanded, and the cut one without options', async () => {
  ctx.sessView.lastMsg = null;
  const s = sess();
  ctx.api = () => Promise.resolve({ sessionId: 'a1', at: 900, text: WHOLE, whole: true });
  vm.runInContext('ensureLastMessage', ctx)(s);
  await new Promise(r => setTimeout(r, 0));
  const expanded = ctx.agentFactsHtml(s, { expanded: true });
  assert.ok(expanded.includes('class="tp-lastmsg"'), expanded);
  assert.ok(expanded.includes('<div class="body">'), expanded);
  assert.ok(expanded.includes(LONG_WORD), 'the long word is intact');
  assert.ok(expanded.includes('line 299') && expanded.includes('END'), 'the last lines are there');
  const plain = ctx.agentFactsHtml(s);
  assert.ok(plain.includes('class="tp-lastmsg"'));
  assert.ok(!plain.includes('END') && !plain.includes('line 299'), 'the panel form is the cut text');
});

test('workHubInfoHtml asks for the whole text, draws the expanded body and drops the expand button', () => {
  detailCalls.length = 0;
  ctx.ensureLastMessage = s => { ensured.push(s); };
  const s = sess();
  const html = vm.runInContext('workHubInfoHtml', ctx)({ id: 'h1' }, s);
  assert.deepStrictEqual(ensured, [s]);
  assert.deepStrictEqual(detailCalls.map(o => ({ ...o })), [{ expanded: true }]);
  assert.ok(!html.includes('data-expand-open'), html);
  assert.ok(html.includes('<p>before</p>') && html.includes('<p>after</p>'), html);
});

test('the last message body in the dialog is a bounded box with its own scroll, after the generic .body rule', () => {
  const css = read('question-card.css').replace(/\/\*[\s\S]*?\*\//g, '');
  const generic = css.indexOf('#question-dialog .body {');
  const rule = css.match(/#question-dialog \.tp-lastmsg > \.body\s*\{([^{}]*)\}/);
  assert.ok(generic >= 0 && rule, 'both rules are found');
  assert.ok(css.indexOf(rule[0]) > generic, 'the last message rule comes after the generic one');
  assert.match(rule[1], /(^|[;\s])overflow\s*:\s*auto/);
  assert.match(rule[1], /(^|[;\s])max-height\s*:/);
  assert.match(rule[1], /overflow-wrap\s*:\s*anywhere/);
});

test('a dialog drawn again puts each last message back where it was scrolled to, by its session', () => {
  vm.runInContext([
    cut(read('sessions-side.js'), /^const lastMessageScrollKey = [\s\S]*?\n\};\n/m),
    cut(read('sessions-side.js'), /^const lastMessageScrolls = [\s\S]*?\);\n/m),
    cut(read('sessions-side.js'), /^function restoreLastMessageScrolls\(.*\) \{[\s\S]*?\n\}/m),
  ].join('\n'), ctx);
  const box = (session, top, at = '1') => ({ parentElement: { dataset: session ? { lastmsgSession: session, lastmsgAt: at } : {} }, scrollTop: top });
  const root = list => ({ querySelectorAll: () => list });
  const tops = vm.runInContext('lastMessageScrolls', ctx)(root([box('w1', 120), box('w2', 40)]));
  // Drawn again in another order, and with one message more: each gets its own place back, the new one stays at the top.
  const again = [box('w2', 0), box('w3', 0), box('w1', 0)];
  vm.runInContext('restoreLastMessageScrolls', ctx)(root(again), tops);
  assert.deepStrictEqual(again.map(b => b.scrollTop), [40, 0, 120]);
  // A new message of the same session is read from its top.
  const newer = [box('w1', 0, '2')];
  vm.runInContext('restoreLastMessageScrolls', ctx)(root(newer), tops);
  assert.strictEqual(newer[0].scrollTop, 0);
});
