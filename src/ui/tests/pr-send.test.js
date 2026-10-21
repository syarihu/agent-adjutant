/* The dialog that sends the comments (src/ui/pr-send.js): 「レビューを送る」 for a review request and 「worker に渡す」 for a task's own read-through,
   against a stand-in dialog, run with `node --test`. What it says is pr-read.js's, loaded with it. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const SHA = 'a'.repeat(40);
const PR = 'acme/widget#1';
const rec = { id: PR, repo: 'acme/widget', number: 1, url: 'https://github.com/acme/widget/pull/1', headSha: SHA, state: 'ai-ready', files: [] };
const line = (key, extra = {}) => ({ key, source: 'person', path: 'a.rs', line: 1, side: 'right', body: `body ${key}`, ...extra });

/* A part of the dialog: markup in, and the box and the chip the code looks for inside the text part. */
function part(inner = {}) {
  return { innerHTML: '', dataset: {}, replaceChildren() { this.innerHTML = ''; }, querySelector: sel => inner[sel] || null, textContent: '' };
}

function world({ kind = 'pr', doc = { version: 1, fromSha: SHA, drafts: [line('p1'), line('p2')], overall: null }, readThrough = { sha: SHA, overall: { body: 'the reader\'s summary' } }, entries = null } = {}) {
  const w = { calls: [], notes: [], doc, read: readThrough, kind, entry: { loaded: true, pending: false, handing: false, pendResult: null, handResult: null, error: null }, active: null, entries, handlers: {}, saved: [] };
  w.box = { value: '', readOnly: false, matches: sel => sel === '.pr-overall', focus() { w.active = w.box; w.calls.push('focus'); } };
  w.chip = { hidden: true };
  w.parts = { text: part({ '.pr-overall': w.box, '[data-pr-send-chip]': w.chip }), rest: part(), foot: part() };
  w.dlg = {
    open: false, returnValue: '', showModal() { this.open = true; w.calls.push('showModal'); }, close() { if (this.open) { this.open = false; w.handlers.close(); } },
    querySelector: sel => (sel === '#review-send-title' ? w.title : sel === '#review-send-text' ? w.parts.text : sel === '#review-send-rest' ? w.parts.rest : sel === '#review-send-foot' ? w.parts.foot : sel === '.pr-overall' ? w.box : null),
    addEventListener: (type, fn) => { w.handlers[type] = fn; },
  };
  w.title = { textContent: '' };
  const c = vm.createContext({
    CSS: { escape: x => x }, console, Promise, setTimeout: () => 0,
    get document() { return { getElementById: id => (id === 'review-send-dialog' ? w.dlg : null), get activeElement() { return w.active; } }; },
    prShown: () => (w.shown === null ? null : { id: PR, rec, kind: w.kind }),
    prReadOf: () => w.read, prDraftsEntry: () => w.entry, prDoc: () => w.doc,
    prShowSave: () => { w.calls.push('save-line'); },
    PR_HOOKS: { diff: { entries: () => w.entries } },
    prPend: (id, rd) => { w.calls.push(['pend', id, rd?.sha]); }, taskReadHand: () => { w.calls.push('hand'); }, prCopy: text => { w.calls.push(['copy', text]); },
    prEditDrafts: (id, rd, fn) => { const d = structuredClone(w.doc); fn(d); w.doc = d; return true; }, prEditRefused: id => { w.calls.push(['refused', id]); },
    prDraftsChanged: id => { w.calls.push(['changed', id]); }, prScheduleSave: (id, now) => { w.saved.push([id, now]); },
    prInput: e => { w.calls.push(['input', e.target.value]); }, prFocusOut: () => {},
  });
  vm.runInContext(['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js', 'pr-send.js'].map(read).join('\n'), c);
  return { w, c, run: e => vm.runInContext(e, c) };
}
const click = (w, sel, extra = {}) => w.handlers.click({ target: { closest: s => (s.split(',').some(x => x.trim() === sel) ? { disabled: false, dataset: {}, ...extra } : null) } });
const sel = (w, c) => ({ text: w.parts.text.innerHTML, rest: w.parts.rest.innerHTML, foot: w.parts.foot.innerHTML });

test('a review request\'s opens as 「レビューを送る」: the overall comment box, what would be sent, and the button that puts it into the pending review', () => {
  const { w, c, run } = world();
  run('prSendOpen()');
  assert.strictEqual(w.title.textContent, 'レビューを送る');
  assert.ok(w.dlg.open && w.calls.includes('showModal'));
  const html = sel(w, c);
  assert.ok(html.text.includes('class="pr-overall" data-pr-overall') && html.text.includes('the reader&#39;s summary') || html.text.includes('the reader\'s summary'), html.text);
  assert.ok(/data-pr-send-chip>AI の文から/.test(html.text), 'the reader\'s own text is marked');
  assert.ok(html.rest.includes('行のコメント 2 件と一緒に、PR の pending review に積みます。'));
  assert.ok(/data-pr-pend>2 件を pending review に積む/.test(html.foot) && html.foot.includes('data-pr-copy-all') && !html.foot.includes('data-pr-hand'));
  assert.ok(html.rest.includes('href="https://github.com/acme/widget/pull/1/files"'));
  // The box takes the keyboard, and the save line is filled in place.
  assert.ok(w.calls.includes('focus') && w.calls.includes('save-line'));
  assert.strictEqual(run('prSend.id'), PR);
});

test('a task\'s opens as 「worker に渡す」, and says nothing of a pending review', () => {
  const { w, c, run } = world({ kind: 'task' });
  run('prSendOpen()');
  assert.strictEqual(w.title.textContent, 'worker に渡す');
  const html = sel(w, c);
  assert.ok(/data-pr-hand>2 件を worker に渡す（全体のコメントも）/.test(html.foot) && !html.foot.includes('data-pr-pend'));
  assert.ok(html.rest.includes('「差し戻す」') && !html.rest.includes('pending review') && !html.rest.includes('GitHub で仕上げる'));
  assert.ok(!html.text.includes('AI の判定の目安'));
});

test('it is not opened for nothing, nor twice, and it says it is loading until the read-through and the drafts are in', () => {
  const none = world();
  none.w.shown = null;
  none.run('prSendOpen()');
  assert.ok(!none.w.dlg.open);
  const { w, c, run } = world();
  w.entry.loaded = false;
  run('prSendOpen()');
  assert.ok(w.dlg.open);
  let html = sel(w, c);
  assert.ok(html.rest.includes('読み込み中…') && html.text === '' && !html.foot.includes('data-pr-pend') && html.foot.includes('閉じる'));
  // The drafts come: the box is drawn then, once.
  w.entry.loaded = true;
  run(`prSendSync('${PR}')`);
  html = sel(w, c);
  assert.ok(html.text.includes('data-pr-overall') && /data-pr-pend>/.test(html.foot));
  const text = html.text;
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.parts.text.innerHTML, text);
  // A failed read of the stored comments is said.
  const failed = world();
  failed.w.entry.loaded = false;
  failed.w.entry.error = 'boom';
  failed.run('prSendOpen()');
  assert.ok(failed.w.parts.rest.innerHTML.includes('保存済みのコメントを読めませんでした（読み直しています）: boom'));
  // The read-through is still on its way.
  const slow = world();
  slow.w.read = undefined;
  slow.run('prSendOpen()');
  assert.ok(slow.w.parts.rest.innerHTML.includes('読み込み中…'));
  // Opened already: another press does nothing.
  w.calls.length = 0;
  run('prSendOpen()');
  assert.deepStrictEqual(w.calls, []);
});

test('the box follows the document without being drawn over while it is typed in, the chip goes once the text is the person\'s, and it is locked while sending', () => {
  const { w, c, run } = world();
  run('prSendOpen()');
  w.box.value = 'my text';
  w.active = w.box;
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.box.value, 'my text', 'being typed in');
  assert.strictEqual(w.chip.hidden, false, 'still the reader\'s untouched text');
  // Typed in: the document has an overall comment, so the chip goes, and the button counts it.
  w.doc = { ...w.doc, overall: { body: 'my text' } };
  w.handlers.input({ target: { matches: s => s === '.pr-overall', value: 'my text' } });
  assert.deepStrictEqual(w.calls.filter(x => Array.isArray(x) && x[0] === 'input'), [['input', 'my text']]);
  assert.strictEqual(w.chip.hidden, true);
  // Not being typed in, what the document says is what the box says (a hand-over clears it).
  w.active = null;
  w.doc = { ...w.doc, overall: { body: '' } };
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.box.value, '');
  assert.ok(/data-pr-pend>2 件を pending review に積む/.test(w.parts.foot.innerHTML));
  // Sending: the box is read-only, the button busy.
  w.entry.pending = true;
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.box.readOnly, true);
  assert.ok(w.parts.foot.innerHTML.includes('aria-busy="true"') && w.parts.foot.innerHTML.includes('積んでいます…'));
  w.entry.pending = false;
  w.entry.pendResult = { sent: 2, failed: [], skipped: [], overall: 'none' };
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.box.readOnly, false);
  assert.ok(w.parts.rest.innerHTML.includes('2 件を積みました'));
  // The sync of another PR's drafts is not this dialog's.
  const before = w.parts.rest.innerHTML;
  w.entry.pendResult = null;
  run("prSendSync('acme/widget#9')");
  assert.strictEqual(w.parts.rest.innerHTML, before);
});

test('the comments the diff cannot show are listed, and none are while the diff is on its way; a task\'s with no file are the PR\'s', () => {
  const doc = { version: 1, fromSha: SHA, overall: { body: '' }, drafts: [line('p1'), line('p2', { path: undefined, line: undefined }), line('p3', { path: 'gone.rs' })] };
  const files = [{ path: 'a.rs', i: 0, file: { all: [{ kind: 'add', new: 1, k: 0 }] } }];
  const on = world({ doc, entries: files });
  on.run('prSendOpen()');
  assert.ok(on.w.parts.rest.innerHTML.includes('差分に出せないコメント（2）') && on.w.parts.rest.innerHTML.includes('data-pr-discard="p2"') && on.w.parts.rest.innerHTML.includes('data-pr-discard="p3"') && !on.w.parts.rest.innerHTML.includes('data-pr-discard="p1"'));
  // No diff to show (none stored, too big): every comment with a file is off it. Still on its way: not known to be.
  const none = world({ doc, entries: null });
  none.run('prSendOpen()');
  assert.ok(none.w.parts.rest.innerHTML.includes('差分に出せないコメント（3）'));
  const loading = world({ doc });
  loading.w.entries = undefined;
  loading.run('prSendOpen()');
  assert.ok(loading.w.parts.rest.innerHTML.includes('差分に出せないコメント（1）') && !loading.w.parts.rest.innerHTML.includes('data-pr-discard="p3"'));
  // A split proposal has no diff: its comments are off it.
  const split = world({ doc, entries: files, readThrough: { sha: SHA, mode: 'split', overall: { body: 'plan' } } });
  split.run('prSendOpen()');
  assert.ok(split.w.parts.rest.innerHTML.includes('差分に出せないコメント（3）'));
  const worker = world({ doc, entries: files, kind: 'task' });
  worker.run('prSendOpen()');
  assert.ok(worker.w.parts.rest.innerHTML.includes('PR 全体へのコメント（1）') && worker.w.parts.rest.innerHTML.includes('差分に出せないコメント（1）'));
});

test('the buttons: push, hand over, copy everything with the overall comment shown, and delete one that is off the diff', () => {
  const { w, run } = world({ doc: { version: 1, fromSha: SHA, overall: null, drafts: [line('p1'), line('p2', { path: undefined, line: undefined })] }, entries: [] });
  run('prSendOpen()');
  w.calls.length = 0;
  click(w, '[data-pr-pend]');
  assert.deepStrictEqual(w.calls, [['pend', PR, SHA]]);
  w.calls.length = 0;
  click(w, '[data-pr-pend]', { disabled: true });
  click(w, '[data-pr-hand]', { disabled: true });
  assert.deepStrictEqual(w.calls, []);
  click(w, '[data-pr-hand]');
  assert.deepStrictEqual(w.calls, ['hand']);
  w.calls.length = 0;
  click(w, '[data-pr-copy-all]');
  assert.strictEqual(w.calls[0][1], 'a.rs:1\nbody p1\n\nPR 全体\nbody p2\n\n全体のコメント:\nthe reader\'s summary');
  w.calls.length = 0;
  click(w, '[data-pr-discard]', { dataset: { prDiscard: 'p2' } });
  assert.deepStrictEqual(w.doc.drafts.map(d => d.key), ['p1']);
  assert.deepStrictEqual(w.calls, [['changed', PR]]);
});

test('closing saves at once and draws what shows the comments again; it closes when the PR on show is another, and is silent when closed', () => {
  const { w, run } = world();
  run('prSendOpen()');
  w.dlg.close();
  assert.deepStrictEqual(w.saved, [[PR, true]]);
  assert.ok(w.calls.some(x => Array.isArray(x) && x[0] === 'changed'));
  assert.strictEqual(run('prSend.id'), null);
  // A sync of a dialog that is closed does nothing.
  w.calls.length = 0;
  run(`prSendSync('${PR}')`);
  assert.deepStrictEqual(w.calls, []);
  // The PR on show moves on while it is open: it closes.
  run('prSendOpen()');
  w.shown = null;
  run(`prSendSync('${PR}')`);
  assert.strictEqual(w.dlg.open, false);
  assert.strictEqual(w.saved.length, 2);
  // A click or an input while nothing is open is nobody's.
  w.calls.length = 0;
  click(w, '[data-pr-pend]');
  assert.deepStrictEqual(w.calls, []);
});
