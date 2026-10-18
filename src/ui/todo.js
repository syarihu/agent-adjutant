/* ── TODO: what waits on the person, the pure half (src/ui/todo-view.js draws and wires it) ────────────────────────────────────
   One list over every repository, in the order it is best answered: first the quick questions that one tap settles, then the ones
   that have to be read before they are decided (a start check, a research result, a plan, a report, a diff), then the pull requests
   the person was asked to review. These functions take the documents and return the list and its markup, touching neither the page
   nor the network, so `src/ui/tests/todo.test.js` runs them under `node --test`. Loaded after question-card.js (`quickRowHtml`,
   `QUESTION_ASK_OF_KIND`), my-work-seen.js (`workEntries`), my-work-others.js (`OTHERS_LIVE`) and aide-display.js (`aideWaits`,
   `aideFocusLine`). */

/* The order the gates to read are taken in: a start check, then the filing, the research or report, the plan, a question, what is
   to go to Jules, the check by hand, and the diff last. A kind not named here goes after them. */
const TODO_READ_ORDER = ['dispatch', 'issue', 'result', 'plan', 'question', 'relay', 'verify', 'diff'];
const todoKindRank = kind => {
  const i = TODO_READ_ORDER.indexOf(kind);
  return i < 0 ? TODO_READ_ORDER.length : i;
};
/* A gate with no time is last, not first: nothing says it has waited. */
const todoAt = item => (Number.isFinite(item.since) ? item.since : Infinity);
const todoOldest = (a, b) => (todoAt(a) === todoAt(b) ? 0 : todoAt(a) < todoAt(b) ? -1 : 1);

/* The key a pull request is selected under; a gate's is its `<board>/<id>`. */
const todoPrKey = rec => `pr:${rec.id}`;

/* What waits on the person: `{ quick, read, prs, count }`. `doc` is the work document and `records` the review records; `isDone(key)`
   says a gate was answered in this page, and `inScope(nwo)` whether a repository is one the list is for. Gates of a parked task are
   left out (`aideWaits`), and every pull request in a state that waits on the person counts, whether or not its AI 下読み has begun. */
function todoItems({ doc, records, isDone = () => false, inScope = () => true }) {
  const quickKeys = new Set();
  for (const repo of doc?.repos || []) {
    for (const t of repo.turns || []) for (const g of t.gates || []) if (g.quick) quickKeys.add(`${g.slug}/${g.id}`);
  }
  const waits = aideWaits(workEntries(doc, isDone)).filter(w => inScope(w.nwo)).map(w => ({ ...w, quick: quickKeys.has(w.key) }));
  const quick = waits.filter(w => w.quick).sort(todoOldest);
  const read = waits.filter(w => !w.quick).sort((a, b) => todoKindRank(a.kind) - todoKindRank(b.kind) || todoOldest(a, b));
  const prs = (records || []).filter(r => OTHERS_LIVE.includes(r.state) && inScope(r.repo)).sort(othersRowOrder);
  return { quick, read, prs, count: quick.length + read.length + prs.length };
}

/* The keys of what is read on the right, in the order the list shows them. */
const todoSelectable = items => [...items.read.map(w => w.key), ...items.prs.map(todoPrKey)];

/* What is selected after the list changed: the same one while it is still there; else the one that has taken its place (the next, or
   the last when it was the last); with nothing selected before, the first. */
function todoPickSelection(sel, before, after) {
  if (sel && after.includes(sel)) return sel;
  const at = sel ? before.indexOf(sel) : -1;
  if (at < 0) return after[0] ?? null;
  return after[Math.min(at, after.length - 1)] ?? null;
}

const todoWaited = (since, now) => (Number.isFinite(since) ? `${agoLabel(minutesSince(since, now))}から待ち` : '待ち');

/* A quick question as its row, with where it is from. Until the board it is on has been read there is no question to show, and no button. */
function todoQuickRowHtml(item, g, ctx = {}, now = Date.now() / 1000, error = null) {
  const from = `<div class="todo-from">${esc(item.nwo)} · ${esc(todoWaited(item.since, now))}</div>`;
  if (!g) return `<div class="todo-quick todo-pending" data-todo-key="${esc(item.key)}"><div class="qc-quick"><b class="qc-quick-text">${esc(item.title)}</b><span class="todo-loading">${error ? esc(`このボードを読めませんでした: ${error}`) : '読み込み中…'}</span></div>${from}</div>`;
  return `<div class="todo-quick" data-todo-key="${esc(item.key)}">${quickRowHtml(g, { ...ctx, key: item.key })}<div class="todo-task">${esc(item.title)}</div>${from}</div>`;
}

/* A gate to read as a button that selects it: what kind, which task, what is asked (the worker's own words, else the kind's fixed question). */
function todoReadRowHtml(item, g, sel, now = Date.now() / 1000) {
  const [label, color] = kindOf(item.kind);
  const ask = aideFocusLine(g?.focus) || QUESTION_ASK_OF_KIND[item.kind] || QUESTION_ASK_OF_KIND.question;
  return `<button type="button" class="todo-row" data-todo-sel="${esc(item.key)}"${sel === item.key ? ' aria-current="true"' : ''}>`
    + `<span class="todo-kind"><i style="background:${esc(color)}"></i>${esc(label)}</span>`
    + `<span class="todo-title">${esc(item.title)}</span><span class="todo-ask">${esc(ask)}</span>`
    + `<span class="todo-from">${esc(item.nwo)} · ${esc(todoWaited(item.since, now))}</span></button>`;
}

/* A pull request waiting on the person's review, as a button that selects it. */
function todoPrRowHtml(rec, sel, now = Date.now() / 1000, records = []) {
  const key = todoPrKey(rec);
  const stage = OTHERS_STAGE_MARK[othersAiStage(rec, records, now)];
  const asked = othersRequestedLabel(rec, now);
  const author = rec.author ? `@${rec.author}` : '';
  return `<button type="button" class="todo-row" data-todo-sel="${esc(key)}"${sel === key ? ' aria-current="true"' : ''}>`
    + `<span class="todo-kind"><i class="todo-pr-dot"></i>レビュー依頼</span>`
    + `<span class="todo-title">#${esc(rec.number)} ${esc(rec.title)}</span><span class="todo-ask">${esc(stage)}</span>`
    + `<span class="todo-from">${esc([rec.repo, author, asked].filter(Boolean).join(' · '))}</span></button>`;
}

/* The whole list: the quick rows, then (only when there is something to read) a separator, the gates to read and the pull requests.
   `gateOf(key)` is the gate once its board has been read, `errorOf(key)` why that read failed, `ctxOf(g)` the card's context, `sel` the selected key. */
function todoListHtml(items, { gateOf = () => null, errorOf = () => null, ctxOf = () => ({}), sel = null, now = Date.now() / 1000, records = [] } = {}) {
  if (!items.count) return '<p class="todo-empty">あなたの番はありません</p>';
  const quick = items.quick.map(w => { const g = gateOf(w.key); return todoQuickRowHtml(w, g, g ? ctxOf(g) : {}, now, g ? null : errorOf(w.key)); }).join('');
  const heavy = items.read.length + items.prs.length > 0;
  return quick
    + (heavy && items.quick.length ? '<h3 class="todo-sep">読んで決める</h3>' : heavy ? '<h3 class="todo-sep first">読んで決める</h3>' : '')
    + items.read.map(w => todoReadRowHtml(w, gateOf(w.key), sel, now)).join('')
    + items.prs.map(r => todoPrRowHtml(r, sel, now, records)).join('');
}
