/* The 「依頼する」 dialog (src/ui/request-dialog.js), run with `node --test`.
   The file binds to the page as it loads, so the pure helpers are cut out of the source and run here. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'request-dialog.js'), 'utf8');
const cut = re => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const plain = v => JSON.parse(JSON.stringify(v));
const ctx = vm.createContext({});
vm.runInContext([
  cut(/^function requestHubOptions[\s\S]*?^}/m),
  cut(/^const requestRepos = [^\n]*;/m),
  cut(/^function requestPickBoard[\s\S]*?^}/m),
  cut(/^const requestBody = [^\n]*;/m),
  cut(/^const requestSendKey = [^\n]*;/m),
].join('\n'), ctx);
const run = (code, vars = {}) => plain(Object.assign(ctx, vars) && vm.runInContext(code, ctx));
const b = (slug, nwo, extra = {}) => ({ slug, nwo, hub: null, finished: false, ...extra });
const boards = [
  b('w-p2', 'acme/widget', { hub: 'ALPHA-2', title: 'Second' }), b('w', 'Acme/Widget'), b('w-p1', 'acme/widget', { hub: 'ALPHA-1' }),
  b('w-old', 'acme/widget', { hub: 'OLD-1', finished: true }), b('t', 'acme/tool'), b('z', 'zeta/z'),
];

test('the hubs of a repository: its own first, then the parents by key, and a finished one is out', () => {
  assert.deepStrictEqual(run('requestHubOptions(boards, "acme/widget")', { boards }), [
    { slug: 'w', label: 'リポジトリ hub' }, { slug: 'w-p1', label: '親 hub · ALPHA-1' }, { slug: 'w-p2', label: '親 hub · ALPHA-2 Second' }]);
  assert.deepStrictEqual(run('requestHubOptions(boards, "nobody/none")', { boards }), []);
});

test('the repositories to ask are the organisation\'s that have a board to take the request', () => {
  const orgs = [{ login: 'acme', repos: ['acme/tool', 'acme/widget', 'acme/gone'] }, { login: 'zeta', repos: ['zeta/z'] }];
  assert.deepStrictEqual(run('requestRepos(orgs, boards, "acme")', { orgs, boards }), ['acme/tool', 'acme/widget']);
  assert.deepStrictEqual(run('requestRepos(orgs, boards, "none")', { orgs, boards }), []);
});

test('the board it opens on: the one named, else the address\'s, else the filter\'s repository, else the first', () => {
  const pick = o => run('requestPickBoard(boards, o)?.slug ?? null', { boards, o });
  assert.strictEqual(pick({ slug: 'w-p1', navBoard: 't' }), 'w-p1');
  assert.strictEqual(pick({ navBoard: 't' }), 't');
  // A finished board is not a place to ask; the work view keeps to the organisation shown.
  assert.strictEqual(pick({ navBoard: 'w-old', org: 'acme' }), 'w');
  assert.strictEqual(pick({ navBoard: 'z', inWork: true, org: 'acme' }), 'w');
  assert.strictEqual(pick({ navBoard: 'z', inWork: false, org: 'acme' }), 'z');
  assert.strictEqual(pick({ org: 'acme', workRepo: 'acme/tool' }), 't');
  assert.strictEqual(pick({ org: 'zeta' }), 'z');
  assert.strictEqual(run('requestPickBoard([], {})'), null);
});

test('the request is the person\'s words, needing reading, with the parent only when there is one', () => {
  assert.deepStrictEqual(run('requestBody("do it", "queued")'), { body: 'do it', status: 'queued', needsReading: true });
  assert.deepStrictEqual(run('requestBody("do it", "backlog", "https://x/1")'), { body: 'do it', status: 'backlog', needsReading: true, parent: 'https://x/1' });
});

test('Ctrl or ⌘ with Enter sends, a plain Enter does not, and an IME\'s Enter never does', () => {
  const send = e => run('requestSendKey(e)', { e });
  assert.strictEqual(send({ key: 'Enter', ctrlKey: true }), true);
  assert.strictEqual(send({ key: 'Enter', metaKey: true }), true);
  assert.strictEqual(send({ key: 'Enter' }), false);
  assert.strictEqual(send({ key: 'a', ctrlKey: true }), false);
  assert.strictEqual(send({ key: 'Enter', ctrlKey: true, isComposing: true }), false);
  assert.strictEqual(send({ key: 'Enter', metaKey: true, keyCode: 229 }), false);
});

/* ── a send that outlives its dialog ── */

const sendCtx = vm.createContext({});
vm.runInContext([
  cut(/^const requestBody = [^\n]*;/m), cut(/^let requestBusy = false;/m), cut(/^let requestDraft = '';/m), cut(/^let requestParent = null;/m),
  cut(/^async function requestSend[\s\S]*?^}/m),
].join('\n'), sendCtx);
const fakeLayer = (text, hub = 'w') => {
  const el = { text: { value: text }, hub: { value: hub }, error: { textContent: '' }, buttons: [{ dataset: { reqSend: 'queued' }, disabled: false }] };
  return { el, querySelector: sel => ({ '[data-req-text]': el.text, '[data-req-hub]': el.hub, '[data-req-error]': el.error })[sel], querySelectorAll: () => el.buttons };
};
function sendSetup() {
  const w = { current: null, closed: 0, notes: [], pending: null };
  Object.assign(sendCtx, {
    dialogStack: { layerEl: () => w.current, closeAll: () => { w.closed++; w.current = null; } },
    boardApi: () => new Promise((ok, no) => { w.pending = { ok, no }; }),
    note: (...a) => w.notes.push(a), handedNote: () => '', refreshWork: () => {}, refresh: () => {},
  });
  vm.runInContext('requestBusy = false; requestDraft = ""; requestParent = null;', sendCtx);
  return w;
}
const draft = () => vm.runInContext('requestDraft', sendCtx);

test('a send that finishes in its own dialog closes it and clears the draft', async () => {
  const w = sendSetup();
  const layer = w.current = fakeLayer('do it');
  vm.runInContext('requestDraft = "do it"', sendCtx);
  const done = sendCtx.requestSend('queued');
  assert.ok(layer.el.buttons[0].disabled);
  w.pending.ok({});
  await done;
  assert.strictEqual(w.closed, 1);
  assert.strictEqual(draft(), '');
});

test('a send that finishes after the dialog was closed and opened again leaves the new dialog open, and its text alone', async () => {
  const w = sendSetup();
  w.current = fakeLayer('do it');
  const done = sendCtx.requestSend('queued');
  // Closed (which keeps the text as the draft), then opened again and typed in.
  vm.runInContext('requestDraft = "do it"', sendCtx);
  const again = w.current = fakeLayer('something new');
  w.pending.ok({});
  await done;
  assert.strictEqual(w.closed, 0);
  assert.strictEqual(again.el.text.value, 'something new');
  assert.strictEqual(draft(), '');
  // Left as it was opened, the text is what was sent and goes.
  const w2 = sendSetup();
  w2.current = fakeLayer('do it');
  const done2 = sendCtx.requestSend('queued');
  const reopened = w2.current = fakeLayer('do it');
  w2.pending.ok({});
  await done2;
  assert.strictEqual(reopened.el.text.value, '');
});

test('a send that fails after its dialog was replaced says so in a note and not in the new dialog', async () => {
  const w = sendSetup();
  w.current = fakeLayer('do it');
  const done = sendCtx.requestSend('queued');
  const again = w.current = fakeLayer('do it');
  w.pending.no(new Error('nope'));
  await done;
  assert.strictEqual(again.el.error.textContent, '');
  assert.ok(w.notes.some(n => n[1] === true));
  assert.ok(!again.el.buttons[0].disabled);
  // In its own dialog the line is there and the text stays.
  const w2 = sendSetup();
  const own = w2.current = fakeLayer('do it');
  const done2 = sendCtx.requestSend('queued');
  w2.pending.no(new Error('nope'));
  await done2;
  assert.strictEqual(own.el.error.textContent, 'nope');
  assert.strictEqual(own.el.text.value, 'do it');
});

test('the text is not given the focus as the dialog opens, so it starts small', () => {
  assert.ok(!/\[data-req-text\]'\)\?\.focus\(\)/.test(src));
});
