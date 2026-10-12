/* The agent's last message in the task panel (lastMessageHtml in src/ui/sessions-side.js), run with `node --test`. */
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

const ctx = vm.createContext({ state: { now: 1000 }, minutesSince: (a, b) => (b - a) / 60, agoLabel: m => `${m}分前` });
vm.runInContext([
  read('util.js'),
  cut(read('card-dialog.js'), /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  cut(read('sessions-side.js'), /^function lastMessageHtml\(.*\) \{[\s\S]*?\n\}/m),
].join('\n'), ctx);
const { lastMessageHtml } = ctx;

test('the last message is drawn as Markdown in a .body', () => {
  const html = lastMessageHtml({ lastMessage: '**x**' });
  assert.ok(html.includes('最後のメッセージ'));
  assert.ok(html.includes('<div class="body">'), html);
  assert.ok(html.includes('<b>x</b>'), html);
});

test('the last message is never HTML', () => {
  const html = lastMessageHtml({ lastMessage: '<script>alert(1)</script>' });
  assert.ok(!html.includes('<script'), html);
  assert.ok(html.includes('&lt;script'), html);
});

test('no message draws nothing', () => {
  assert.strictEqual(lastMessageHtml({ lastMessage: '' }), '');
  assert.strictEqual(lastMessageHtml({}), '');
});

/* The whole text, asked for when the expanded view opens (ensureLastMessage, lastMessageBodyHtml). */
const asked = [];
const arrived = [];
let reply = () => Promise.resolve({});
const wctx = vm.createContext({
  state: { now: 1000 }, minutesSince: (a, b) => (b - a) / 60, agoLabel: m => `${m}分前`,
  sessView: { lastMsg: null }, enc: encodeURIComponent,
  api: url => { asked.push(url); return reply(url); },
  lastMessageArrived: () => { arrived.push(1); },
});
vm.runInContext([
  read('util.js'),
  cut(read('card-dialog.js'), /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  cut(read('sessions-side.js'), /^const LAST_MESSAGE_SHOWN_CHARS = [^\n]*\n/m),
  cut(read('sessions-side.js'), /^const lastMessageKey = [^\n]*\n/m),
  cut(read('sessions-side.js'), /^const lastMessageCut = [\s\S]*?\n\};\n/m),
  cut(read('sessions-side.js'), /^function ensureLastMessage\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function forgetLastMessage\(\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function lastMessageBodyHtml\(.*\) \{[\s\S]*?\n\}/m),
  cut(read('sessions-side.js'), /^function lastMessageHtml\(.*\) \{[\s\S]*?\n\}/m),
].join('\n'), wctx);
const w = code => vm.runInContext(code, wctx);
const CUT = `${'x'.repeat(1000)}…`;
const sess = (extra = {}) => ({ id: 'w1', present: true, agentSession: { sessionId: 'a1', lastMessage: CUT, lastMessageAt: 900, ...extra } });
const settle = () => new Promise(r => setTimeout(r, 0));
const fresh = () => { asked.length = 0; arrived.length = 0; wctx.sessView.lastMsg = null; reply = () => Promise.resolve({}); };

test('the normal view is the cut text and names its session; the expanded view is the whole text', async () => {
  fresh();
  const s = sess();
  const normal = wctx.lastMessageHtml(s.agentSession, s);
  assert.ok(normal.includes('data-lastmsg-session="w1"'), normal);
  assert.ok(normal.includes(CUT), normal);
  assert.ok(!normal.includes('lastmsg-note'));
  // Without a session it is what it always was.
  assert.ok(!wctx.lastMessageHtml(s.agentSession).includes('data-lastmsg-session'));
  const whole = `${'y'.repeat(3000)} END`;
  reply = () => Promise.resolve({ sessionId: 'a1', at: 900, text: whole, whole: true });
  w('ensureLastMessage')(s);
  await settle();
  assert.deepStrictEqual(asked, ['/api/sessions/w1/message']);
  const expanded = wctx.lastMessageHtml(s.agentSession, s, { expanded: true });
  assert.ok(expanded.includes('END'), expanded);
  assert.ok(!expanded.includes('lastmsg-note'));
  assert.strictEqual(arrived.length, 1);
  // The normal view is unchanged by it.
  assert.ok(!wctx.lastMessageHtml(s.agentSession, s).includes('END'));
});

test('the cut text and a note stand in while it loads, when it is gone and when it fails', async () => {
  fresh();
  const s = sess();
  let release;
  reply = () => new Promise(r => { release = r; });
  w('ensureLastMessage')(s);
  let html = wctx.lastMessageHtml(s.agentSession, s, { expanded: true });
  assert.ok(html.includes('全文を読み込んでいます…') && html.includes(CUT), html);
  assert.ok(html.includes('role="status"'));
  release({ sessionId: 'a1', at: 900, text: CUT, whole: false });
  await settle();
  html = wctx.lastMessageHtml(s.agentSession, s, { expanded: true });
  assert.ok(html.includes('全文は残っていないため、先頭の 1000 文字までを表示しています'), html);
  assert.ok(html.includes(CUT));

  fresh();
  reply = () => Promise.reject(new Error('no such <agent> session'));
  w('ensureLastMessage')(s);
  await settle();
  html = wctx.lastMessageHtml(s.agentSession, s, { expanded: true });
  assert.ok(html.includes('全文を読み込めませんでした: no such &lt;agent&gt; session（先頭の 1000 文字までを表示しています）'), html);
  assert.ok(!html.includes('<agent>'));
});

test('an answer for another message or session is dropped, and a key is asked only once', async () => {
  fresh();
  const s = sess();
  reply = () => Promise.resolve({ sessionId: 'a1', at: 800, text: 'older', whole: true });
  w('ensureLastMessage')(s);
  await settle();
  assert.ok(!wctx.lastMessageHtml(s.agentSession, s, { expanded: true }).includes('older'));
  fresh();
  reply = () => Promise.resolve({ sessionId: 'other', at: 900, text: 'other', whole: true });
  w('ensureLastMessage')(s);
  await settle();
  assert.ok(!wctx.lastMessageHtml(s.agentSession, s, { expanded: true }).includes('other'));
  // The same message again is not asked again; a new one is.
  w('ensureLastMessage')(s);
  w('ensureLastMessage')(s);
  assert.strictEqual(asked.length, 1);
  w('ensureLastMessage')(sess({ lastMessageAt: 950 }));
  assert.strictEqual(asked.length, 2);
});

test('nothing is asked for a session with no agent session, no message time or a message that was not cut', () => {
  fresh();
  w('ensureLastMessage')(null);
  w('ensureLastMessage')({ id: 'w1', present: false, agentSession: sess().agentSession });
  w('ensureLastMessage')(sess({ sessionId: undefined }));
  w('ensureLastMessage')(sess({ lastMessageAt: null }));
  w('ensureLastMessage')(sess({ lastMessage: 'short and whole' }));
  assert.deepStrictEqual(asked, []);
  // A row cut by an older binary is asked for.
  w('ensureLastMessage')(sess());
  assert.strictEqual(asked.length, 1);
});

test('opening the expanded view asks afresh, and a redraw does not', async () => {
  fresh();
  const s = sess();
  reply = () => Promise.reject(new Error('down'));
  w('ensureLastMessage')(s);
  await settle();
  // The redraw that follows the failure asks nothing more.
  w('ensureLastMessage')(s);
  assert.strictEqual(asked.length, 1);
  reply = () => Promise.resolve({ sessionId: 'a1', at: 900, text: 'the whole', whole: true });
  w('forgetLastMessage')();
  w('ensureLastMessage')(s);
  await settle();
  assert.strictEqual(asked.length, 2);
  assert.ok(wctx.lastMessageHtml(s.agentSession, s, { expanded: true }).includes('the whole'));
  w('ensureLastMessage')(s);
  assert.strictEqual(asked.length, 2);
  // Opened again: a loaded one is asked for again, as its tail may have changed under the same time.
  reply = () => Promise.resolve({ sessionId: 'a1', at: 900, text: 'the new tail', whole: true });
  w('forgetLastMessage')();
  w('ensureLastMessage')(s);
  await settle();
  assert.strictEqual(asked.length, 3);
  assert.ok(wctx.lastMessageHtml(s.agentSession, s, { expanded: true }).includes('the new tail'));
});
