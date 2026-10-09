/* The 「拡大」 dialog (src/ui/card-dialog.js) and what it relies on in decide.js, run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const dlgSrc = read('card-dialog.js');
const decSrc = read('decide.js');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const calls = [];
const ctx = vm.createContext({
  esc, gateRef: g => g._slug ? `${g._slug}/${g.id}` : g.id, calls,
  answerResult: true, closeResult: true, answerImpl: null,
  gateByRef: id => ({ id }), boardApi: async () => { throw new Error('x'); },
  baseOf: () => '', note() {}, gateKey: () => 'k', gateAnswered() {}, dropGate() {}, refresh: async () => {},
  refreshBoards() {}, talk: id => { calls.push(['talk', id]); },
  commentBox: () => null, baseName: x => x, handedNote: () => '',
  document: { querySelectorAll: () => [], querySelector: () => null },
});
vm.runInContext([
  cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
  cut(dlgSrc, /^const expandAttrs = [\s\S]*?;\nconst expandBtnHtml = [\s\S]*?;\n/m),
  cut(dlgSrc, /^const cardDialogPanelCards = [^\n]*;/m),
  cut(dlgSrc, /^function cardDialogFocusTarget\(\) \{[\s\S]*?\n\}/m),
  cut(decSrc, /^const deciding = new Set\(\);[^\n]*\n/m),
  'async function answer(...a) { calls.push(["answer", ...a]); return answerResult; }',
  cut(decSrc, /^async function decideAct\(b\) \{[\s\S]*?\n\}/m),
  cut(decSrc, /^async function closeGate\(id\) \{[\s\S]*?\n\}/m),
].join('\n'), ctx);
const run = code => vm.runInContext(code, ctx);

test('expandBtnHtml escapes the label and is a plain button with no data-action', () => {
  const h = run(`expandBtnHtml('<b>"x"</b>')`);
  assert.ok(h.includes('type="button"'));
  assert.ok(!h.includes('<b>'));
  assert.ok(h.includes('data-expand-open'));
  assert.ok(!h.includes('data-action'));
  assert.ok(/<span class="material-symbols-outlined" aria-hidden="true">open_in_full<\/span>/.test(h));
});

test('expandAttrs escapes the gate ref and leaves it out with no gate', () => {
  assert.strictEqual(run(`expandAttrs('report')`), ' data-expand="report"');
  assert.strictEqual(run(`expandAttrs('report', { id: 'a"b', _slug: 's' })`), ' data-expand="report" data-expand-gate="s/a&quot;b"');
});

test('cardDialogFocusTarget falls back from the opener to the card, the tab and the close button', () => {
  const el = (shown, extra = {}) => ({ isConnected: shown, getClientRects: () => shown ? [1] : [], focus() {}, ...extra });
  const set = (opener, again, tab, close) => {
    ctx.opener = opener;
    ctx.document.querySelectorAll = () => again ? [{ dataset: { expand: 's', expandGate: 'g' }, querySelector: () => again }] : [];
    ctx.document.querySelector = sel => sel.includes('aria-selected') ? tab : close;
    run(`cardDialog.opener = opener; cardDialog.section = 's'; cardDialog.gate = 'g';`);
    return run('cardDialogFocusTarget()');
  };
  const o = el(true), a = el(true), t = el(true), c = el(true);
  assert.strictEqual(set(o, a, t, c), o);
  assert.strictEqual(set(el(false), a, t, c), a);
  assert.strictEqual(set(el(false), el(false), t, c), t);
  assert.strictEqual(set(el(false), null, el(false), c), c);
  assert.strictEqual(set(null, null, null, el(false)), null);
});

const btn = (gate, extra = {}) => ({ closest: () => ({ dataset: { gate } }), matches: () => !!extra.choice, dataset: { act: extra.act, choice: extra.choice } });

test('decideAct returns whether the answer went, and false for a second click in flight', async () => {
  calls.length = 0;
  ctx.answerResult = true;
  assert.strictEqual(await run(`decideAct`)(btn('g1', { act: 'approve' })), true);
  ctx.answerResult = false;
  assert.strictEqual(await run(`decideAct`)(btn('g1', { choice: 'c1' })), false);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(calls.map(c => c.slice(0, 3)))), [['answer', 'approve', null], ['answer', 'choice', 'c1']]);
  ctx.answerResult = true;
  let release;
  ctx.answerImpl = new Promise(r => { release = r; });
  vm.runInContext('answer = async () => answerImpl', ctx);
  const first = run(`decideAct`)(btn('g2', { act: 'approve' }));
  assert.strictEqual(await run(`decideAct`)(btn('g2', { act: 'approve' })), false);
  release(true);
  assert.strictEqual(await first, true);
});

test('talk does not count as an answer', async () => {
  assert.strictEqual(await run(`decideAct`)(btn('g3', { act: 'talk' })), false);
});

test('closeGate returns false when the board refuses, and true when it took it', async () => {
  assert.strictEqual(await run(`closeGate('g')`), false);
  ctx.boardApi = async () => ({});
  assert.strictEqual(await run(`closeGate('g')`), true);
  ctx.gateByRef = () => null;
  assert.strictEqual(await run(`closeGate('g')`), false);
});

test('the panel cards of a gate carry the attributes the dialog finds them by', () => {
  const c2 = vm.createContext({
    esc, gateRef: g => g.id, expandAttrs: run('expandAttrs'), expandBtnHtml: run('expandBtnHtml'), OUTCOME: { open: ['未対応', 0], fixed: ['修正済', 1], declined: ['誤検知', 2] },
    manualChecked: () => new Set(), md: esc, kindOf: () => ['x'],
  });
  vm.runInContext([
    ...['reviewPanels', 'roundsCardHtml', 'findingsCardHtml', 'checkPanels', 'commandsCardHtml', 'manualCardHtml']
      .map(n => cut(decSrc, new RegExp(`^function ${n}\\(g\\) \\{[\\s\\S]*?\\n\\}`, 'm'))),
    cut(decSrc, /^function choicesHtml\(g, pickable\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c2);
  const g = { id: 'g1', reviewRounds: [{ engine: 'e' }], findings: [{ outcome: 'open', severity: 'must', text: 't' }],
    commands: [{ command: 'c', result: 'pass' }], manual: ['m'], choices: [{ id: 'a', label: 'A' }] };
  c2.g = g;
  const all = vm.runInContext('reviewPanels(g) + checkPanels(g) + choicesHtml(g, true)', c2);
  for (const s of ['rounds', 'findings', 'commands', 'manual', 'choices']) {
    assert.ok(all.includes(`data-expand="${s}" data-expand-gate="g1"`), s);
  }
  assert.strictEqual(all.match(/data-expand-open/g).length, 5);
});

test('card-dialog.css undoes the generic dialog .body and sets display only when open', () => {
  const css = read('card-dialog.css').replace(/\/\*[\s\S]*?\*\//g, '');
  const rule = sel => {
    const m = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(x => x[1].trim() === sel);
    assert.ok(m, sel);
    return m[2];
  };
  const body = rule('#card-dialog .body');
  assert.match(body, /padding:\s*0/);
  assert.match(body, /display:\s*block/);
  assert.match(body, /overflow:\s*visible/);
  assert.match(rule('#card-dialog .body table'), /overflow-x:\s*auto/);
  assert.doesNotMatch(rule('#card-dialog'), /display\s*:/);
  assert.match(rule('#card-dialog[open]'), /display:\s*flex/);
});

/* cardDialogSync against a small stand-in for the DOM it reads. */
function syncWorld({ source, controls = [], subject = 's1', hidden = false, pane = 'detail', focused = false, expect = true }) {
  const live = { source, controls };
  const gateEl = () => {
    const buttons = [];
    for (let i = 0; i < 2; i++) buttons.push({ remove() { buttons.splice(buttons.indexOf(this), 1); } });
    const ta = { value: 'typed', readOnly: false };
    return { attrs: { 'data-gate': 'g' }, buttons, ta, removeAttribute(a) { delete this.attrs[a]; },
      setAttribute(a, v) { this.attrs[a] = v; }, hasAttribute(a) { return a in this.attrs; },
      querySelectorAll(sel) { return sel === 'button' ? [...buttons] : sel === 'textarea' ? [ta] : []; } };
  };
  const gates = [gateEl()];
  const goneEl = { textContent: '' };
  const body = { scrollTop: 0 };
  const content = { drawn: null, get children() { return [{ hasAttribute: () => false }, ...gates]; }, replaceChildren(...a) { this.drawn = a; }, querySelectorAll: sel => sel === '[data-gate]' ? gates.filter(g => g.attrs['data-gate']) : [] };
  const dlg = {
    open: true, closed: false, close() { this.open = false; this.closed = true; },
    querySelector: sel => sel === '.card-dialog-gone' ? goneEl : sel === '.card-dialog-body' ? body : null,
  };
  const c = vm.createContext({
    selectedTaskId: subject, nav: { pane }, tp: () => ({ hidden }),
    document: { activeElement: focused ? { matches: () => true } : null },
    cardDialogEl: () => dlg, cardDialogContent: () => content, cardDialogBox: () => gates[0].ta,
    cardDialogPanelComment: () => null, cardDialogClone: x => x,
    cardDialogSource: () => live.source, cardDialogControls: () => live.controls,
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};\nconst CARD_DIALOG_GONE = [^\n]*;\nconst CARD_DIALOG_GATE_GONE = [^\n]*;/m),
    cut(dlgSrc, /^function cardDialogStrip\(text\) \{[\s\S]*?\n\}/m),
    cut(dlgSrc, /^function cardDialogStripEl\(el\) \{[\s\S]*?\n\}/m),
    cut(dlgSrc, /^function cardDialogNotice\(text\) \{[\s\S]*?\n\}/m),
    cut(dlgSrc, /^function cardDialogSync\([^)]*\) \{[\s\S]*?\n\}/m),
    `Object.assign(cardDialog, { subject: 's1', pane: 'detail', section: 'report', gate: 'g', expectControls: ${expect}, sig: 'old' });`,
  ].join('\n'), c);
  return { c, dlg, goneEl, live, gates, content, sync: (...a) => { c.args = a; return vm.runInContext('cardDialogSync(...args)', c); }, state: () => vm.runInContext('cardDialog', c) };
}
const card = (gate = null) => ({ dataset: { gate }, outerHTML: '<c>' });

test('a source card that left the panel strips the answer controls and says so', () => {
  const w = syncWorld({ source: null });
  w.sync();
  assert.notStrictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.gates[0].attrs['data-gate'], undefined);
  assert.strictEqual(w.gates[0].buttons.length, 0);
  assert.strictEqual(w.gates[0].ta.readOnly, true);
  assert.strictEqual(w.state().sig, '');
});

test('while a comment is typed, a gate that left the panel still strips the controls and keeps the comment', () => {
  const w = syncWorld({ source: card(), controls: [], focused: true });
  w.sync();
  assert.notStrictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.gates[0].attrs['data-gate'], undefined);
  assert.strictEqual(w.gates[0].ta.value, 'typed');
  assert.strictEqual(w.dlg.closed, false);
});

test('a comment being typed holds the redraw while the gate is still there', () => {
  const w = syncWorld({ source: card(), controls: [card('g')], focused: true });
  w.sync();
  assert.strictEqual(w.state().held, true);
  assert.strictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.gates[0].attrs['data-gate'], 'g');
});

test('another subject, a hidden panel or another pane closes the dialog quietly', () => {
  for (const o of [{ subject: 'other' }, { hidden: true }, { pane: 'review' }]) {
    const w = syncWorld({ source: card(), controls: [card('g')], ...o });
    w.sync();
    assert.strictEqual(w.dlg.closed, true, JSON.stringify(o));
    assert.strictEqual(w.state().quiet, true);
  }
});

test('cardDialogSource takes a card only when exactly one in the panel matches', () => {
  const c = vm.createContext({ document: { querySelectorAll: () => [] } });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^const cardDialogPanelCards = [^\n]*;/m),
    cut(dlgSrc, /^function cardDialogSource\([^)]*\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c);
  const el = (expand, gate) => ({ dataset: { expand, expandGate: gate } });
  const a = el('report', 'g');
  const look = list => { c.document.querySelectorAll = () => list; return vm.runInContext(`cardDialogSource('report', 'g')`, c); };
  assert.strictEqual(look([]), null);
  assert.strictEqual(look([el('report', 'other'), el('facts', 'g')]), null);
  assert.strictEqual(look([a, el('report', 'other')]), a);
  assert.strictEqual(look([a, el('report', 'g')]), null);
});

function openWorld(sources) {
  const notes = [];
  const dlg = { open: false };
  const card = { dataset: { expand: 'report', expandGate: 'g' } };
  const c = vm.createContext({
    note: (...a) => notes.push(a), cardDialogEl: () => dlg, selectedTaskId: 's1', nav: { pane: 'detail' },
    cardDialogSource: () => sources, cardDialogSync() {}, cardDialogContent: () => ({}), cardDialogTask: () => null,
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^function openCardDialog\(btn\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c);
  return { c, notes, btn: { closest: () => card, dataset: {} } };
}

test('opening a card with no unique source leaves the state unset and says so', () => {
  const w = openWorld(null);
  vm.runInContext('globalThis.b = null', w.c);
  w.c.btn = w.btn;
  vm.runInContext('openCardDialog(btn)', w.c);
  assert.strictEqual(w.notes.length, 1);
  assert.strictEqual(w.notes[0][1], true);
  assert.strictEqual(vm.runInContext('cardDialog.subject', w.c), null);
  assert.strictEqual(vm.runInContext('cardDialog.token', w.c), 0);
});

test('a late answer does not close a dialog opened after the click, and a failed one resyncs', async () => {
  let release;
  const state = { open: true, closed: 0, synced: 0 };
  const c = vm.createContext({
    cardDialogEl: () => state, closeCardDialog: () => { state.closed++; state.open = false; },
    cardDialogSync: (...a) => { state.synced++; state.args = a; state.answeringThen = vm.runInContext('cardDialog.answering', c); }, decideAct: () => new Promise(r => { release = r; }),
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^async function cardDialogAnswer\(b\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c);
  // Answered in the same opening: closes.
  let p = vm.runInContext('cardDialogAnswer({})', c);
  assert.strictEqual(vm.runInContext('cardDialog.answering', c), true);
  release(true); await p;
  assert.strictEqual(state.closed, 1);
  // Another opening came in while the answer was on its way: left alone.
  state.open = true;
  p = vm.runInContext('cardDialogAnswer({})', c);
  vm.runInContext('cardDialog.token += 1; cardDialog.answering = false', c); // what a new opening does
  release(true); await p;
  assert.strictEqual(state.closed, 1);
  // A failed answer keeps the dialog and resyncs.
  vm.runInContext('cardDialog.held = true', c);
  p = vm.runInContext('cardDialogAnswer({})', c);
  release(false); await p;
  assert.strictEqual(state.closed, 1);
  assert.strictEqual(state.synced, 1);
  assert.strictEqual(state.args.length, 0, 'not forced');
  assert.strictEqual(state.answeringThen, false, 'cleared before the sync');
  assert.strictEqual(vm.runInContext('cardDialog.held', c), false);
});

test('controls that appear after the opening are stripped too when they vanish', () => {
  const w = syncWorld({ source: card(), controls: [], expect: false });
  w.sync();
  assert.strictEqual(w.goneEl.textContent, '');
  w.live.controls = [card('g')];
  w.sync();
  assert.strictEqual(w.state().expectControls, true);
  assert.strictEqual(w.goneEl.textContent, '');
  w.live.controls = [];
  w.sync();
  assert.notStrictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.gates[0].attrs['data-gate'], undefined);
});

test('a press on a gate button defers the flush of a held redraw until the click is handled', () => {
  const timers = [];
  const synced = [];
  const c = vm.createContext({
    setTimeout: f => timers.push(f), cardDialogSync: () => synced.push(1), document: { activeElement: null },
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^function cardDialogPressEnd\(\) \{[\s\S]*?\n\}/m),
    'cardDialog.held = true;',
  ].join('\n'), c);
  vm.runInContext('cardDialogPressEnd()', c);
  assert.strictEqual(timers.length, 0, 'no press, nothing to do');
  vm.runInContext('cardDialog.pressing = true; cardDialogPressEnd()', c);
  assert.strictEqual(synced.length, 0, 'not before the click has run');
  timers.shift()();
  assert.strictEqual(synced.length, 1);
  assert.strictEqual(vm.runInContext('cardDialog.pressing', c), false);
});

test('only the gate gone: the card is redrawn without the controls and the notice says it was the answer', () => {
  const w = syncWorld({ source: card(), controls: [] });
  const src = w.live.source;
  w.sync();
  assert.match(w.goneEl.textContent, /^この gate への答え/);
  assert.strictEqual(w.gates[0].attrs['data-gate'], undefined);
  // Followed on: the card is drawn again, with the stripped control (and its comment) kept.
  assert.strictEqual(w.content.drawn[0], src);
  assert.strictEqual(w.content.drawn[1], w.gates[0]);
  assert.strictEqual(w.gates[0].ta.value, 'typed');
  // The card itself going is a different notice.
  w.live.source = null;
  w.sync();
  assert.match(w.goneEl.textContent, /^パネルからこのカード/);
});

test('a sync while an answer is on its way does nothing', () => {
  const w = syncWorld({ source: null });
  vm.runInContext('cardDialog.answering = true', w.c);
  w.sync();
  assert.strictEqual(w.goneEl.textContent, '');
  assert.strictEqual(w.gates[0].attrs['data-gate'], 'g');
  vm.runInContext('cardDialog.answering = false', w.c);
  w.sync();
  assert.notStrictEqual(w.goneEl.textContent, '');
});

test('a second click while an answer is on its way returns at once and leaves `answering` set', async () => {
  let release, calls = 0;
  const c = vm.createContext({
    cardDialogEl: () => ({ open: true }), closeCardDialog() {}, cardDialogSync() {},
    decideAct: () => { calls++; return new Promise(r => { release = r; }); },
  });
  vm.runInContext([
    cut(dlgSrc, /^const cardDialog = \{[\s\S]*?\n\};/m),
    cut(dlgSrc, /^async function cardDialogAnswer\(b\) \{[\s\S]*?\n\}/m),
  ].join('\n'), c);
  const first = vm.runInContext('cardDialogAnswer({})', c);
  await vm.runInContext('cardDialogAnswer({})', c);
  assert.strictEqual(calls, 1);
  assert.strictEqual(vm.runInContext('cardDialog.answering', c), true);
  release(true); await first;
  assert.strictEqual(vm.runInContext('cardDialog.answering', c), false);
});

test('the dialog of a task fills the width: a grid of cards two to a row, an index that goes at 720px', () => {
  const raw = read('card-dialog.css');
  const css = raw.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules = sel => [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(x => x[1].trim() === sel).map(x => x[2]).join(' ');
  assert.doesNotMatch(css, /max-width:\s*1080px/);
  const grid = rules('.cd-grid');
  assert.match(grid, /grid-template-columns:[^;]*minmax\(/);
  assert.match(grid, /440px/);
  assert.match(grid, /calc\(\(100% - 16px\) \/ 2\)/);
  assert.match(rules('.cd-wide'), /grid-column:\s*1\s*\/\s*-1/);
  assert.match(rules('.card-dialog-index'), /(?:flex:\s*0 0 232px|width:\s*232px)/);
  assert.match(rules('.card-dialog-index'), /overflow:\s*auto/);
  const media = css.match(/@media \(max-width: 720px\) \{([\s\S]*?\})\s*\}/);
  assert.ok(media);
  assert.match(media[1], /\.card-dialog-index\s*\{\s*display:\s*none/);
  // The size and the open rules of the dialog itself are as before.
  assert.match(rules('#card-dialog'), /width:\s*calc\(100vw - 48px\)/);
  assert.match(rules('#card-dialog'), /height:\s*calc\(100dvh - 48px\)/);
  assert.match(rules('#card-dialog[open]'), /display:\s*flex/);
  assert.match(rules('.cd-dock'), /position:\s*sticky/);
  assert.match(raw, /prefers-reduced-motion/);
});

test('the head of a task has a button for the whole task, in the panel head and with an accessible name', () => {
  const head = cut(read('task-panel.js'), /^function panelHeadHtml\(task\) \{[\s\S]*?\n\}/m);
  assert.match(head, /data-expand-task/);
  assert.match(head, /aria-label="タスク全体を拡大して読む"/);
  assert.match(head, /<span class="material-symbols-outlined" aria-hidden="true">open_in_full<\/span>/);
  assert.ok(!head.includes('task.id'));
  const markup = read('page-body.html');
  assert.match(markup, /<nav class="card-dialog-index" aria-label="目次"/);
  // A group is named, and the clean-up of the drawn cards keeps aria-label (it strips only what points at ids).
  const src = read('card-dialog.js');
  assert.match(src, /class="cd-group" data-cd-group="[^"]*" aria-label="\$\{esc\(g\.label\)\}"/);
  assert.doesNotMatch(cut(src, /^function cardDialogFill\([\s\S]*?\n\}/m), /removeAttribute\('aria-label'\)/);
});
