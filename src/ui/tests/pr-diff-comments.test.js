/* The comments on the lines of the annotated diff (`prdComments` in src/ui/pr-diff-view.js) against a small stand-in for the DOM, run with
   `node --test`: the elements are added, taken away and drawn again to match the drafts' document and the open editors, an element that is being
   typed in is never replaced, the buttons follow the lock and the blank box, and 「コメントにする」 of a concern hides while the reader's comment is on
   its line. The stand-in parses the markup the page makes, so the rows and the comments are the real ones. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const SHA = 'a'.repeat(40);
const OTHER = 'b'.repeat(40);

/* ── a DOM of just what the code uses ── */

const camel = name => name.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
const unescapeHtml = t => t.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');
class Node {
  constructor(tag = 'div', attrs = {}) {
    this.tag = tag;
    this.attrs = { ...attrs };
    this.dataset = {};
    for (const [k, v] of Object.entries(attrs)) if (k.startsWith('data-')) this.dataset[camel(k.slice(5))] = v;
    this.children = [];
    this.parent = null;
    this.text = '';
    this.disabled = false;
    this.hidden = 'hidden' in attrs;
    this.value = '';
    this.focused = 0;
    const classes = () => new Set(String(this.attrs.class || '').split(/\s+/).filter(Boolean));
    this.classList = { contains: c => classes().has(c) };
  }
  get isConnected() { let n = this; while (n.parent) n = n.parent; return n.root === true; }
  get firstElementChild() { return this.children[0] || null; }
  get firstChild() { return this.children[0] || null; }
  get nextElementSibling() { return this.parent ? this.parent.children[this.parent.children.indexOf(this) + 1] || null : null; }
  get nextSibling() { return this.nextElementSibling; }
  get previousElementSibling() { return this.parent ? this.parent.children[this.parent.children.indexOf(this) - 1] || null : null; }
  detach() { if (this.parent) { this.parent.children.splice(this.parent.children.indexOf(this), 1); this.parent = null; } }
  insertBefore(el, ref) { el.detach(); const at = ref ? this.children.indexOf(ref) : this.children.length; this.children.splice(at, 0, el); el.parent = this; return el; }
  appendChild(el) { return this.insertBefore(el, null); }
  after(el) { el.detach(); const p = this.parent; p.children.splice(p.children.indexOf(this) + 1, 0, el); el.parent = p; }
  replaceWith(el) { const p = this.parent; const at = p.children.indexOf(this); this.detach(); el.detach(); p.children.splice(at, 0, el); el.parent = p; }
  remove() { this.detach(); }
  contains(n) { for (let x = n; x; x = x.parent) if (x === this) return true; return false; }
  focus() { this.focused++; world.active = this; }
  toggleAttribute(name, on) { if (name === 'hidden') this.hidden = !!on; }
  setAttribute(k, v) { this.attrs[k] = v; }
  removeAttribute(k) { delete this.attrs[k]; }
  /* One compound selector: tag, .class, [attr], [attr="v"]. */
  hit(sel) {
    return [...sel.matchAll(/^([a-z]+)|\.([\w-]+)|\[([\w-]+)(?:="([^"]*)")?\]/g)].filter(m => m[0]).every(m => {
      if (m[1]) return this.tag === m[1];
      if (m[2]) return this.classList.contains(m[2]);
      return m[4] === undefined ? m[3] in this.attrs : this.attrs[m[3]] === m[4];
    });
  }
  /* A selector list, each with at most one descendant combinator. */
  matches(sel) {
    return sel.split(',').map(x => x.trim()).some(one => {
      const parts = one.split(/\s+/);
      if (!this.hit(parts[parts.length - 1])) return false;
      let anc = this.parent;
      for (let i = parts.length - 2; i >= 0; i--) { while (anc && !anc.hit(parts[i])) anc = anc.parent; if (!anc) return false; anc = anc.parent; }
      return true;
    });
  }
  all(sel) { return this.children.flatMap(c => [...(c.matches(sel) ? [c] : []), ...c.all(sel)]); }
  querySelectorAll(sel) { return this.all(sel); }
  querySelector(sel) { return this.all(sel)[0] || null; }
  closest(sel) { for (let n = this; n; n = n.parent) if (n.matches(sel)) return n; return null; }
}

/* The markup of the page, as nodes: tags, attributes and text; a textarea's text is its value (without the newline the parser drops). */
function parse(html) {
  const top = new Node('#fragment');
  let at = top;
  for (const m of html.matchAll(/<(\/?)([a-zA-Z0-9]+)((?:\s+[\w-]+(?:="[^"]*")?)*)\s*>|([^<]+)/g)) {
    if (m[4] !== undefined) { at.text += unescapeHtml(m[4]); continue; }
    if (m[1]) { if (at.tag === 'textarea') at.value = at.text.replace(/^\n/, ''); at = at.parent; continue; }
    const attrs = {};
    for (const a of m[3].matchAll(/([\w-]+)(?:="([^"]*)")?/g)) attrs[a[1]] = a[2] === undefined ? '' : unescapeHtml(a[2]);
    const n = new Node(m[2], attrs);
    at.appendChild(n);
    at = n;
  }
  return top;
}

const world = { active: null };

/* ── the page, as far as the comments need ── */

const DIFF = [
  'diff --git a/src/a.rs b/src/a.rs', '--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,3 +1,4 @@', ' fn a() {', '-    old();', '+    new();', '+    more();', ' }', '',
].join('\n');
const rec = { id: 'acme/widget#1', repo: 'acme/widget', number: 1, url: 'https://github.com/acme/widget/pull/1', headSha: SHA, files: [] };
const readThrough = (extra = {}) => ({ sha: SHA, madeAt: 'm1', order: [], deleted: [], annotations: [
  { path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'Explains' }, { path: 'src/a.rs', anchor: 'more();', kind: 'concern', text: 'Why?', draft: 'Could you say why?' }], ...extra });

function setup({ doc = { version: 1, fromSha: null, drafts: [], overall: null }, entry = {}, boxes = [] } = {}) {
  world.active = null;
  const w = { doc, entry: { loaded: true, pending: false, handing: false, pendResult: null, ...entry }, boxes: new Map(boxes), shown: { id: rec.id, kind: 'pr' } };
  const c = vm.createContext({
    TextEncoder, TextDecoder, CSS: { escape: x => x }, console, Promise, Date, setTimeout: () => 0, structuredClone,
    get document() { return { activeElement: world.active, createElement: () => ({ set innerHTML(h) { this.content = parse(h); } }) }; },
    prView: { boxes: new Map([[rec.id, w.boxes]]) }, prDraftsEntry: () => w.entry, prDoc: () => w.doc, prShown: () => w.shown,
    secTitle: t => t, DIFF_LOADING: 'loading', PR_HOOKS: { diff: null }, cardDialogContent: () => ({ addEventListener() {} }),
  });
  vm.runInContext(['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js'].map(read).join('\n'), c);
  vm.runInContext(read('pr-diff-view.js'), c);
  const rd = readThrough();
  const files = c.prParseDiff(DIFF);
  const e = { state: 'ok', gen: 1, read: rd, layout: c.prDiffLayout(files, rd, rec), resolved: c.prDiffResolve(files, rd), bodies: new Map() };
  const entryOf = e.layout.files[0];
  // The card of the file, drawn as the page draws it: the shell, and its body filled.
  const root = new Node('div');
  root.root = true;
  const card = parse(c.prDiffShellHtml(entryOf, e.resolved.files.get('src/a.rs'))).firstElementChild;
  root.appendChild(card);
  const body = card.querySelector('[data-prd-body]');
  const fill = () => { const f = parse(c.prDiffBodyHtml(entryOf, e.resolved.files.get('src/a.rs'), rd, true)); [...f.children].forEach(x => body.appendChild(x)); body.dataset.prdFilled = String(e.gen); };
  const keys = () => root.all('[data-prd-cmt]').map(x => x.dataset.prdCmt);
  const run = () => c.prdComments(rec.id, e, root);
  return { w, c, e, root, card, body, fill, keys, run, slot: card.querySelector('[data-prd-fcmts]') };
}
/* The input event of a box: what the page's handler is given, which is the box itself. */
const typed = (t, ta) => t.c.prdInput({ target: { matches: s => s === '[data-prd-box]', dataset: ta.dataset, value: ta.value, closest: s => ta.closest(s) } });
const draft = (key, extra = {}) => ({ key, source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: `body ${key}`, sha: SHA, ...extra });
/* The order of a body's children by what they are: a row by its place, an annotation, a comment. */
const order = body => body.children.map(n => (n.dataset.prdI !== undefined ? `row${n.dataset.prdI}` : n.dataset.prdAnn !== undefined ? `ann${n.dataset.prdAnn}` : n.dataset.prdCmt !== undefined ? `cmt:${n.dataset.prdCmt}` : 'hunk'));

test('a comment is under its line, after the line\'s annotations, before the next line, in the order of the document', () => {
  const t = setup({ doc: { drafts: [draft('p1'), draft('p2'), draft('p3', { line: 2, side: 'left' })] } });
  t.fill();
  // `new();` is new line 2 (k 2, with its explanation) and `more();` new line 3 (k 3, with its concern); `-    old();` is old line 2 (k 1).
  assert.ok(order(t.body).includes('ann0') && order(t.body).includes('ann1'), 'the annotations are in the body before the comments are');
  t.run();
  const o = order(t.body);
  assert.ok(o.indexOf('row3') < o.indexOf('ann1') && o.indexOf('ann1') < o.indexOf('cmt:p1') && o.indexOf('cmt:p1') < o.indexOf('cmt:p2'), o.join(' '));
  assert.strictEqual(o[o.indexOf('cmt:p2') + 1], 'row4', 'before the next line');
  assert.ok(o.indexOf('row1') < o.indexOf('cmt:p3') && o.indexOf('cmt:p3') < o.indexOf('row2'), 'the removed line\'s comment is under the removed line');
  // Nothing else is drawn twice, and nothing is in the slot.
  assert.deepStrictEqual(t.keys().sort(), ['p1', 'p2', 'p3']);
  assert.strictEqual(t.slot.children.length, 0);
});

test('a comment on a file, one whose line is gone and an editor with no line are in the slot under the header', () => {
  const t = setup({ doc: { drafts: [draft('f1', { line: undefined }), draft('g1', { line: 40 }), draft('p1')] }, boxes: [['new:src/a.rs|right|', { kind: 'new', path: 'src/a.rs', side: 'right', line: null, source: 'person', text: '' }]] });
  // The slot is in the header's card, so it is there before the body is drawn.
  t.run();
  assert.deepStrictEqual(t.slot.children.map(x => x.dataset.prdCmt), ['f1', 'g1', 'new:src/a.rs|right|']);
  assert.ok(t.slot.children[1].all('.prd-cmt-lost').length === 1 && t.slot.children[1].all('.prd-cmt-lost')[0].text.includes('行が見つからないコメント（src/a.rs:40）'));
  assert.strictEqual(t.slot.children[0].all('.prd-cmt-lost').length, 0);
  // The line's comment waits for its body (a file not drawn yet gets it when it is).
  assert.deepStrictEqual(t.keys().sort(), ['f1', 'g1', 'new:src/a.rs|right|']);
  t.fill();
  t.run();
  assert.deepStrictEqual(t.keys().sort(), ['f1', 'g1', 'new:src/a.rs|right|', 'p1']);
});

test('only the one file asked for is looked at, and the comments of a body are put in when it is filled', () => {
  const t = setup({ doc: { drafts: [draft('p1')] } });
  t.fill();
  t.c.prdComments(rec.id, t.e, t.root, 5);
  assert.deepStrictEqual(t.keys(), [], 'another file: nothing here');
  t.c.prdComments(rec.id, t.e, t.root, 0);
  assert.deepStrictEqual(t.keys(), ['p1']);
});

test('what did not change is left alone; what changed is drawn again; what is gone or moved goes', () => {
  const t = setup({ doc: { drafts: [draft('p1'), draft('p2')] } });
  t.fill();
  t.run();
  const [p1, p2] = t.root.all('[data-prd-cmt]');
  t.run();
  assert.strictEqual(t.root.all('[data-prd-cmt]')[0], p1, 'the same element');
  // The text changed: that one is a new element, the other stays.
  t.w.doc = { drafts: [draft('p1', { body: 'changed' }), draft('p2')] };
  t.run();
  const [n1, n2] = t.root.all('[data-prd-cmt]');
  assert.notStrictEqual(n1, p1);
  assert.ok(n1.all('.prd-cmt-text')[0].text === 'changed');
  assert.strictEqual(n2, p2);
  // Gone: removed. Moved to another line: the element goes under that line.
  t.w.doc = { drafts: [draft('p1', { body: 'changed', line: 2 })] };
  t.run();
  assert.deepStrictEqual(t.keys(), ['p1']);
  const o = order(t.body);
  assert.ok(o.indexOf('row2') < o.indexOf('cmt:p1') && o.indexOf('cmt:p1') < o.indexOf('row3'), o.join(' '));
  // The head it was written on is said only when it differs.
  assert.strictEqual(t.root.all('.prd-chip-warn').length, 0);
  t.w.doc = { drafts: [draft('p1', { sha: OTHER, line: 2 })] };
  t.run();
  assert.strictEqual(t.root.all('.prd-chip-warn').length, 1);
  assert.ok(t.root.all('.prd-chip-warn')[0].text.includes('bbbbbbb の差分で書きました'));
});

test('an element whose box has the keyboard is never replaced, and a blank box cannot be added', () => {
  const box = { kind: 'new', path: 'src/a.rs', side: 'right', line: 3, source: 'person', text: '', focus: true };
  const t = setup({ boxes: [['new:src/a.rs|right|3', box]] });
  t.fill();
  t.run();
  const el = t.root.all('[data-prd-cmt]')[0];
  const ta = el.querySelector('textarea');
  // A new editor takes the keyboard once.
  assert.strictEqual(ta.focused, 1);
  assert.strictEqual(box.focus, false);
  t.run();
  assert.strictEqual(ta.focused, 1);
  // Blank: 「コメントを足す」 is off. Typed in: on, by the page's own handler, with no redraw.
  const add = el.querySelector('[data-prd-cmt-act="add"]');
  assert.strictEqual(add.disabled, true);
  ta.value = 'something';
  typed(t, ta);
  assert.strictEqual(add.disabled, false);
  assert.strictEqual(box.text, 'something');
  // Another thing changed in the element's markup (the reader's chip) while it has the keyboard: it stays as it is.
  world.active = ta;
  box.source = 'reader';
  t.run();
  assert.strictEqual(t.root.all('[data-prd-cmt]')[0], el);
  assert.strictEqual(el.querySelector('textarea').value, 'something');
  // Once the keyboard is elsewhere it is drawn again, with what is typed.
  world.active = null;
  t.run();
  const next = t.root.all('[data-prd-cmt]')[0];
  assert.notStrictEqual(next, el);
  assert.strictEqual(next.querySelector('textarea').value, 'something');
  assert.ok(next.all('.pr-chip').some(x => x.text === 'AI の文から'));
  // A comment that is gone is taken away even with the keyboard in its box.
  world.active = next.querySelector('textarea');
  t.w.boxes.clear();
  t.run();
  assert.deepStrictEqual(t.keys(), []);
  world.active = null;
});

test('an editor for a saved comment shows its text and keeps what is typed; 保存 follows the box', () => {
  const t = setup({ doc: { drafts: [draft('p1')] }, boxes: [['p1', { kind: 'edit', text: 'old text', focus: true }]] });
  t.fill();
  t.run();
  const el = t.root.all('[data-prd-cmt]')[0];
  assert.ok(el.classList.contains('prd-cmt-edit'));
  assert.strictEqual(el.querySelector('textarea').value, 'old text');
  assert.strictEqual(el.querySelector('[data-prd-cmt-act="save"]').disabled, false);
  el.querySelector('textarea').value = '  ';
  typed(t, el.querySelector('textarea'));
  assert.strictEqual(el.querySelector('[data-prd-cmt-act="save"]').disabled, true);
  // The box is closed (by 「やめる」, so the keyboard is on that button): the saved text is back.
  world.active = null;
  t.w.boxes.clear();
  t.run();
  const back = t.root.all('[data-prd-cmt]')[0];
  assert.ok(back.classList.contains('prd-cmt-saved') && back.all('.prd-cmt-text')[0].text === 'body p1');
});

test('while the comments are on their way to a pending review or a worker every button and box is off, and it follows the lock', () => {
  const t = setup({ doc: { drafts: [draft('p1'), draft('p2', { sentAt: '20261001T000000Z' })] }, boxes: [['new:src/a.rs|right|2', { kind: 'new', path: 'src/a.rs', side: 'right', line: 2, source: 'person', text: 'x' }]] });
  t.fill();
  t.run();
  const controls = () => t.root.all('[data-prd-cmt] button, [data-prd-cmt] textarea');
  assert.ok(controls().length >= 6);
  assert.strictEqual(controls().filter(x => x.disabled).length, 0);
  t.w.entry.pending = true;
  t.run();
  assert.ok(controls().every(x => x.disabled));
  t.w.entry.pending = false;
  t.w.entry.handing = true;
  t.run();
  assert.ok(controls().every(x => x.disabled));
  t.w.entry.handing = false;
  t.run();
  assert.strictEqual(controls().filter(x => x.disabled).length, 0);
  // One that was sent has no 編集, and 削除 says what it does not do.
  const sent = t.root.all('[data-prd-cmt]').find(x => x.dataset.prdCmt === 'p2');
  assert.strictEqual(sent.all('[data-prd-cmt-act="edit"]').length, 0);
  assert.ok(sent.all('[data-prd-cmt-act="delete"]')[0].attrs.title.includes('GitHub の pending review や worker への指示は消えません'));
});

test('why a push left a comment out is said on it, and a task\'s comments say 渡しました', () => {
  const t = setup({ doc: { drafts: [draft('p1'), draft('p2', { sentAt: '20261001T000000Z' })] }, entry: { pendResult: { sent: 0, failed: [{ key: 'p1', why: 'Line must be part of the diff' }], skipped: [], overall: 'none' } } });
  t.fill();
  t.run();
  assert.ok(t.root.all('.pr-why')[0].text.includes('積めませんでした: Line must be part of the diff'));
  assert.ok(t.root.all('.pr-chip-sent')[0].text === '積みました');
  const w2 = setup({ doc: { drafts: [draft('p2', { sentAt: '20261001T000000Z' })] } });
  w2.w.shown = { id: rec.id, kind: 'task' };
  w2.fill();
  w2.run();
  assert.ok(w2.root.all('.pr-chip-sent')[0].text === '渡しました');
});

test('「コメントにする」 of a concern is hidden while a comment of the reader\'s is on its line, and back when it is gone', () => {
  const t = setup({ doc: { drafts: [draft('p1', { source: 'person' })] } });
  t.fill();
  t.run();
  const concern = t.body.all('.prd-ann-concern')[0];
  const btn = concern.querySelector('[data-prd-todraft]');
  assert.strictEqual(btn.hidden, false);
  // A comment of the reader's on that line (same path, side and line): the concern cannot be added twice.
  t.w.doc = { drafts: [draft('p1', { source: 'reader' })] };
  t.run();
  assert.strictEqual(btn.hidden, true);
  // On another line, or by the person: it is not that concern's.
  t.w.doc = { drafts: [draft('p1', { source: 'reader', line: 2 }), draft('p2', { source: 'person' })] };
  t.run();
  assert.strictEqual(btn.hidden, false);
  // The explanation's 「コメント」 is never hidden.
  t.w.doc = { drafts: [draft('p1', { source: 'reader', line: 2 })] };
  t.run();
  assert.strictEqual(t.body.all('.prd-ann-ex')[0].querySelector('[data-prd-todraft]').hidden, false);
});

test('comments are put in only while the diff is the current one', () => {
  const t = setup({ doc: { drafts: [draft('p1')] } });
  t.fill();
  t.e.state = 'loading';
  t.run();
  assert.deepStrictEqual(t.keys(), []);
  t.e.state = 'ok';
  t.run();
  assert.deepStrictEqual(t.keys(), ['p1']);
});
