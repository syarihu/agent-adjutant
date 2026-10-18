/* ── TODO: the page's half (the list and its markup are todo.js; the card is question-card.js) ───────────────────────────────────
   The sidebar's third entry counts what waits on the person, shows it in a popover while the pointer or the keyboard is on the entry,
   and opens a view of it: the list on the left (quick questions answered where they stand, then what is read before it is decided, then
   the pull requests to review) and the selected question as the same card the dialog has on the right. The work document and the review
   records are the page's own (`work`, `others`); a gate's content is not in them, so the state of each board that has a gate is read
   here (`todo.boards`) and handed to the card through a question source (question-dialog.js). Loaded after question-dialog.js,
   my-work.js, my-work-others-view.js and pr-panel.js, and before main.js. */

/* Every organisation until the sidebar picks one: the one place that narrows the TODO. */
const todoInScope = nwo => true;

/* How long what was read of a board is kept before a poll reads it again. */
const TODO_BOARD_FRESH_MS = 10000;
/* The pointer rests on the entry this long before the popover opens, and has left both this long before it closes. */
const TODO_POP_OPEN_MS = 150;
const TODO_POP_CLOSE_MS = 250;

const todo = {
  boards: {},      // slug -> { data (the board's state, its lists tagged with the board), at, error }
  boardSeq: {},    // slug -> the number of the last read sent
  boardApplied: {},// slug -> the number of the last read applied
  items: null,     // what `todoMine` said at the last round
  keys: [],        // what can be selected, as last drawn
  sel: null,       // the selected key (a gate's, or a pull request's): the page's, not the address's
  html: { pop: null, list: null, card: null },
  card: null,      // { key, head, body } of the gate drawn on the right
  popOpen: false,
  over: { tab: false, pop: false },
  timer: null,
};

const todoMine = () => todoItems({
  doc: work.doc, records: othersRecords(), isDone: key => answeredGates.has(key), inScope: todoInScope,
});

const todoSlugOf = key => key.slice(0, Math.max(key.lastIndexOf('/'), 0));

/* The gate as its board last said it, or null while the board has not been read or no longer lists it as waiting. */
function todoGateOf(key) {
  const g = (todo.boards[todoSlugOf(key)]?.data?.gates || []).find(x => gateKey(x) === key);
  return g && g.wait !== false ? g : null;
}
registerQuestionSource(key => {
  const g = answeredGates.has(key) ? null : todoGateOf(key);
  return g ? { g, data: todo.boards[todoSlugOf(key)].data } : null;
});

/* Why the board of `key` could not be read, or null. */
const todoBoardError = key => todo.boards[todoSlugOf(key)]?.error || null;

/* The boards that have a gate in the list are read, while the list is on screen or about to be. */
const todoBoardsWanted = () => view === 'todo' || todo.popOpen;

/* Read the state of each board that has a gate (without its sessions, as 「すべて」 does), unless it was read a moment ago and `force`
   does not say otherwise. A board that cannot be read keeps what was read of it, with the reason. Each read draws as it arrives. */
async function todoLoadBoards(force = false) {
  if (!multiBoard) return;
  const items = todoMine();
  const slugs = new Set([...items.quick, ...items.read].map(w => w.board));
  const at = Date.now();
  const stale = [...slugs].filter(slug => force || !todo.boards[slug] || at - todo.boards[slug].at >= TODO_BOARD_FRESH_MS);
  await Promise.all(stale.map(async slug => {
    const n = todo.boardSeq[slug] = (todo.boardSeq[slug] || 0) + 1;
    // A read sent before a newer one was applied (or before an answer took a gate off) must not put it back.
    const current = () => {
      if (n < (todo.boardApplied[slug] || 0)) return false;
      todo.boardApplied[slug] = n;
      return true;
    };
    try {
      const next = await boardApi(`/b/${slug}`, '/api/state?sessions=0');
      if (!current()) return;
      // The lists `mergeStates` (core.js) tags with their board, since the cards match on `_slug`.
      const tagged = Object.fromEntries(['tasks', 'gates', 'waits', 'parents', 'workers', 'hubs'].map(k => [k, (next[k] || []).map(x => ({ ...x, _slug: slug, _base: `/b/${slug}` }))]));
      todo.boards[slug] = { data: { ...next, ...tagged }, at: Date.now(), error: null };
    } catch (e) {
      if (!current()) return;
      todo.boards[slug] = { data: todo.boards[slug]?.data || null, at: Date.now(), error: e.message };
    }
    todoFollow();
  }));
}

/* ── Drawing ── */

/* Where the focus is inside `el`, as something that can be found again after `el` is drawn anew. */
function todoFocusMark(el) {
  const a = document.activeElement;
  if (!a || !el.contains(a)) return null;
  if (a.dataset.todoSel) return `[data-todo-sel="${CSS.escape(a.dataset.todoSel)}"]`;
  const root = a.closest('[data-qc-gate]');
  const what = a.dataset.choice ? `[data-choice="${CSS.escape(a.dataset.choice)}"]` : a.dataset.act ? `[data-act="${CSS.escape(a.dataset.act)}"]` : '';
  return root && what ? `[data-qc-gate="${CSS.escape(root.dataset.qcGate)}"] ${what}` : null;
}

/* `html` into `el` when it is not what is there, the focus kept on the same button. */
function todoSetHtml(slot, el, html) {
  if (todo.html[slot] === html) return;
  todo.html[slot] = html;
  const mark = todoFocusMark(el);
  el.innerHTML = html;
  if (mark) el.querySelector(mark)?.focus({ preventScroll: true });
}

const todoCtxOf = g => ({ ...questionCtxFor(g), key: gateKey(g) });

/* Why the list cannot be trusted to be whole, as a line: a read that failed is not an empty list. */
const todoWorkNote = () => !work.doc ? (work.error ? `読み込めませんでした: ${work.error}` : '読み込み中…') : '';
const todoOthersNote = () => others.error ? `<p class="todo-hint">レビュー依頼を読めませんでした: ${esc(others.error)}</p>` : '';

function todoListOf(items, sel) {
  // Nothing has been read yet, or the read failed: not the same as nothing waiting.
  if (!work.doc) return `<p class="todo-hint">${esc(todoWorkNote())}</p>`;
  return todoOthersNote() + todoListHtml(items, { gateOf: todoGateOf, errorOf: todoBoardError, sel, now: Date.now() / 1000, records: othersRecords() });
}

function todoDrawBadge(items) {
  const badge = document.getElementById('todo-count');
  if (!badge) return;
  badge.textContent = items.count;
  badge.classList.toggle('zero', items.count === 0);
}

function todoDrawList(items) {
  const el = document.getElementById('todo-list');
  todoSetHtml('list', el, `<h2 class="todo-head">あなたの番<span class="rail-badge${items.count ? '' : ' zero'}">${items.count}</span></h2>`
    + `<div class="todo-scroll">${todoListOf(items, todo.sel)}</div>`);
}

/* The card the right side shows for a pull request: where it is from, how far its AI 下読み is, and the way into its review. */
function todoPrCardHtml(rec) {
  const records = othersRecords();
  const now = othersNowEst();
  const stage = OTHERS_STAGE_MARK[othersAiStage(rec, records, now)];
  const url = httpUrl(rec.url);
  const sub = [rec.repo, rec.author ? `@${rec.author}` : '', othersRequestedLabel(rec, now)].filter(Boolean).map(x => `<span class="qc-where">${esc(x)}</span>`).join('');
  return `<article class="qc todo-article" data-todo-pr="${esc(rec.id)}"><header class="qd-head"><div class="qd-head-main">`
    + `<div class="qd-sub"><span class="qc-kind"><i class="todo-pr-dot"></i>レビュー依頼</span>${sub}</div><h2 class="qd-title">#${esc(rec.number)} ${esc(rec.title)}</h2></div></header>`
    + `<div class="qd-body"><section class="qc-stage"><h3><span class="material-symbols-outlined" aria-hidden="true">menu_book</span><span>AI 下読み</span></h3><p class="todo-stage">${esc(stage)}</p></section></div>`
    + `<footer class="qd-foot"><div class="decide qc-decide"><button type="button" class="btn-m3-primary" data-todo-pr-open="${esc(rec.id)}">レビューの画面で開く</button>`
    + `<span class="qc-spacer"></span>${url ? `<a class="qc-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer"><span class="material-symbols-outlined" aria-hidden="true">open_in_new</span><span>GitHub で開く</span></a>` : ''}</div></footer></article>`;
}

function todoEmptyCardHtml(items) {
  if (!work.doc) return `<div class="todo-card-empty"><p class="todo-hint">${esc(todoWorkNote())}</p></div>`;
  const hint = items.quick.length ? '左の確認は、その場で押せます。' : 'worker が何か聞いてきたら、ここに出ます。';
  return `<div class="todo-card-empty">${todoOthersNote()}<p class="todo-empty">あなたの番はありません</p><p class="todo-hint">${esc(hint)}</p></div>`;
}

/* The gate's card: the head when it changed, the body that scrolls when it changed and nothing in it has the focus (keeping where it was
   scrolled to), and the foot only when another gate is selected, so that what is typed in it is never lost. */
function todoDrawGateCard(el, key, g) {
  const ctx = todoCtxOf(g);
  const c = questionCardHtml(g, { ...ctx, draft: questionDrafts.get(key) || '' });
  const shown = todo.card?.key === key && el.querySelector('article[data-qc-gate]');
  if (!shown) {
    todo.html.card = null;
    el.innerHTML = `<article class="qc todo-article" data-qc-gate="${esc(key)}"><header class="qd-head"><div class="qd-head-main">${c.head}</div></header>`
      + `<div class="qd-body" tabindex="0" role="region" aria-label="内容">${c.body}</div><footer class="qd-foot">${c.foot}</footer></article>`;
    todo.card = { key, head: c.head, body: c.body };
    return;
  }
  if (c.head !== todo.card.head) {
    el.querySelector('.qd-head-main').innerHTML = c.head;
    todo.card.head = c.head;
  }
  const body = el.querySelector('.qd-body');
  if (c.body === todo.card.body || (document.activeElement !== body && body.contains(document.activeElement))) return;
  const top = body.scrollTop;
  body.innerHTML = c.body;
  body.scrollTop = top;
  todo.card.body = c.body;
}

function todoDrawCard(items) {
  const el = document.getElementById('todo-card');
  const key = todo.sel;
  if (!key) { todo.card = null; return todoSetHtml('card', el, todoEmptyCardHtml(items)); }
  if (key.startsWith('pr:')) {
    const rec = othersById(key.slice(3));
    todo.card = null;
    return todoSetHtml('card', el, rec ? todoPrCardHtml(rec) : todoEmptyCardHtml(items));
  }
  const g = todoGateOf(key);
  if (!g) {
    todo.card = null;
    const why = todoBoardError(key);
    return todoSetHtml('card', el, `<div class="todo-card-empty"><p class="todo-hint">${why ? esc(`このボードを読めませんでした: ${why}`) : '読み込み中…'}</p></div>`);
  }
  todoDrawGateCard(el, key, g);
}

function todoDrawPop(items) {
  const pop = document.getElementById('todo-pop');
  todoSetHtml('pop', pop, `<button type="button" class="todo-pop-head" data-todo-open><b>あなたの番 ${items.count}</b><span>押すと TODO を開く</span></button>`
    + `<div class="todo-scroll">${todoListOf(items, null)}</div>`);
  todoPlace();
}

/* What is drawn follows what is known: the badge on every view, the popover while it is open, the list and the card in the TODO view.
   Each part is drawn only when its markup changed; `force` draws it anyway. */
function todoFollow(force = false) {
  if (!multiBoard) return;
  const items = todo.items = todoMine();
  todoDrawBadge(items);
  if (force) todo.html = { pop: null, list: null, card: null };
  const keys = todoSelectable(items);
  todo.sel = todoPickSelection(todo.sel, todo.keys, keys);
  todo.keys = keys;
  if (view === 'todo') {
    if (todo.popOpen) todoPopClose();
    todoDrawList(items);
    todoDrawCard(items);
  } else if (todo.popOpen) {
    todoDrawPop(items);
  }
}

registerView('todo-view', { render: () => todoFollow() });

/* Called every round, whichever view is on: the boards of the list are read only while it is shown, and the badge follows the documents. */
async function todoPoll() {
  if (todoBoardsWanted()) await todoLoadBoards();
  todoFollow();
}

/* ── Opening the view ── */

/* The TODO view, with `sel` selected when it is given. */
function todoOpen(sel = null) {
  if (sel) todo.sel = sel;
  if (view !== 'todo') go({ board: 'all', view: 'todo', task: null, pane: 'detail' });
  else todoFollow(true);
}

function todoSelect(key) {
  todo.sel = key;
  todoFollow(true);
}

/* ── The popover ── */

const todoTabEl = () => document.getElementById('nav-todo');
const todoPopEl = () => document.getElementById('todo-pop');
const todoInside = el => !!el && (todoTabEl().contains(el) || todoPopEl().contains(el));

/* Under the entry, or beside it when the sidebar is only icons; kept inside the window. */
function todoPlace() {
  const pop = todoPopEl();
  if (pop.hidden) return;
  const r = todoTabEl().getBoundingClientRect();
  const icons = document.body.classList.contains('rail-icons');
  const w = Math.min(380, window.innerWidth - 16);
  pop.style.width = `${w}px`;
  const h = pop.offsetHeight;
  const left = icons ? r.right + 8 : r.left;
  const top = icons ? r.top : r.bottom + 4;
  pop.style.left = `${Math.max(8, Math.min(left, window.innerWidth - w - 8))}px`;
  pop.style.top = `${Math.max(8, Math.min(top, window.innerHeight - h - 8))}px`;
}

function todoPopOpen() {
  clearTimeout(todo.timer);
  // Not in the TODO view, which has the list itself, and not under a dialog.
  if (todo.popOpen || view === 'todo' || questionDialogEl().open || !multiBoard) return;
  todo.popOpen = true;
  todoPopEl().hidden = false;
  todo.html.pop = null;
  todoFollow();
  todoLoadBoards();
}

function todoPopClose({ focusTab = false } = {}) {
  clearTimeout(todo.timer);
  if (!todo.popOpen) return;
  const had = todoPopEl().contains(document.activeElement);
  todo.popOpen = false;
  todoPopEl().hidden = true;
  todo.html.pop = null;
  if (focusTab || had) todoTabEl().focus({ preventScroll: true });
}

/* The popover opens a moment after the pointer arrives, and closes a moment after it has left both the entry and the popover. */
function todoPopSchedule() {
  clearTimeout(todo.timer);
  if (todo.over.tab || todo.over.pop) {
    if (!todo.popOpen) todo.timer = setTimeout(todoPopOpen, TODO_POP_OPEN_MS);
  } else if (todo.popOpen) {
    todo.timer = setTimeout(() => { if (!todo.over.tab && !todo.over.pop && !todoInside(document.activeElement)) todoPopClose(); }, TODO_POP_CLOSE_MS);
  }
}

for (const [name, el] of [['tab', todoTabEl()], ['pop', todoPopEl()]]) {
  el.addEventListener('pointerenter', () => {
    todo.over[name] = true;
    // The wish to look is the hint to read the boards, so the questions are in by the time it is open.
    if (name === 'tab' && view !== 'todo') { todoLoadBoards(); }
    todoPopSchedule();
  });
  el.addEventListener('pointerleave', () => { todo.over[name] = false; todoPopSchedule(); });
}
todoTabEl().addEventListener('focus', () => { if (todoTabEl().matches(':focus-visible')) { todoLoadBoards(); todoPopOpen(); } });
// Focus moving on to something outside both closes it. A click that moves no focus (nothing focused is under the pointer) is not a move.
document.addEventListener('focusout', e => {
  if (todo.popOpen && e.relatedTarget && todoInside(e.target) && !todoInside(e.relatedTarget)) todoPopClose();
});
document.addEventListener('pointerdown', e => { if (todo.popOpen && !todoInside(e.target)) todoPopClose(); });
document.addEventListener('keydown', e => {
  // An open dialog takes Escape, whichever it is.
  if (e.key !== 'Escape' || !todo.popOpen || e.isComposing || e.keyCode === 229 || document.querySelector('dialog[open]')) return;
  e.preventDefault();
  todoPopClose({ focusTab: true });
});
window.addEventListener('resize', todoPlace);

/* ── Clicks ── */

function todoClick(e) {
  const row = e.target.closest('[data-todo-sel]');
  if (row) return e.currentTarget.id === 'todo-pop' ? todoOpen(row.dataset.todoSel) : todoSelect(row.dataset.todoSel);
  if (e.target.closest('[data-todo-open]')) return todoOpen();
  const pr = e.target.closest('[data-todo-pr-open]');
  if (pr) {
    const rec = othersById(pr.dataset.todoPrOpen);
    return rec ? prOpen(rec) : note('開く', true, 'このレビュー依頼は、いま読めません');
  }
  const open = e.target.closest('[data-qc-open]');
  if (open) return questionOpenLayer(open.dataset.qcOpen, open);
  const b = e.target.closest('.pick[data-choice], [data-act], [data-qc-ide]');
  if (!b || b.disabled) return;
  const key = b.closest('[data-qc-gate]')?.dataset.qcGate;
  if (b.dataset.act === 'talk') {
    const g = key && questionLive(key);
    return g ? go({ board: g._slug, view: 'work', task: g.task || null, pane: 'detail' }) : note('話す', true, 'この質問は、もう待っていません');
  }
  questionAct(b).then(() => todoFollow(true));
}
for (const id of ['todo-pop', 'todo-list', 'todo-card']) document.getElementById(id).addEventListener('click', todoClick);

// What is typed in the card is kept for the next time it is opened, as the dialog keeps it.
document.getElementById('todo-card').addEventListener('input', e => {
  const box = e.target.closest?.('.gate-comment');
  const key = box?.closest('[data-qc-gate]')?.dataset.qcGate;
  if (!key) return;
  if (box.value) questionDrafts.set(key, box.value); else questionDrafts.delete(key);
});
