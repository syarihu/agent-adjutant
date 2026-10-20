/* ── TODO: what waits on the person, the pure half (src/ui/todo-view.js draws and wires it) ────────────────────────────────────
   One list over every repository, in the order it is best answered: first the questions that one tap settles (a merged session to clean
   up, a quick question of a worker), then the ones
   that have to be read before they are decided (a start check, a research result, a plan, a report, a diff). The pull requests the
   person was asked to review are not here: they are 「レビュー」's. These functions take the documents and return the list and its markup, touching neither the page
   nor the network, so `src/ui/tests/todo.test.js` runs them under `node --test`. Loaded after question-card.js (`quickRowHtml`,
   `QUESTION_ASK_OF_KIND`), my-work-clean.js (`workMergedAskOf`, `workMergedAskHtml`), my-work-seen.js (`workEntries`, `workWaits`) and gate-evidence.js (`gateFocusLine`). */

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

/* What waits on the person: `{ clean, quick, read, count }`. `doc` is the work document; `isDone(key)` says a gate was answered in this
   page, and `inScope(nwo)` whether a repository is one the list is for; `kept` is the keys of the sessions 「残す」 was pressed on and
   the document does not yet say so for. Gates of a parked task are left out (`workWaits`). `clean` is a merged session the board asks
   about by itself (`workMergedAskOf`, in my-work-clean.js): not a gate, so nothing reads a board for it. Oldest first by the time the session
   reached its last phase (`phaseAt`), those with none after, by title. */
function todoItems({ doc, isDone = () => false, inScope = () => true, kept = new Set() }) {
  const quickKeys = new Set();
  for (const repo of doc?.repos || []) {
    for (const t of repo.turns || []) for (const g of t.gates || []) if (g.quick) quickKeys.add(`${g.slug}/${g.id}`);
  }
  const entries = workEntries(doc, isDone);
  const waits = workWaits(entries).filter(w => inScope(w.nwo)).map(w => ({ ...w, quick: quickKeys.has(w.key) }));
  const quick = waits.filter(w => w.quick).sort(todoOldest);
  const read = waits.filter(w => !w.quick).sort((a, b) => todoKindRank(a.kind) - todoKindRank(b.kind) || todoOldest(a, b));
  const clean = [];
  for (const e of entries) {
    if (!inScope(e.nwo)) continue;
    const a = workMergedAskOf(e, { isDone, kept });
    if (a?.ask !== 'clean') continue;
    clean.push({ key: workAskKey(e.board, e.session.id), board: e.board, sessionId: e.session.id, nwo: e.nwo, title: a.title, text: workMergedAskText(a, true),
      files: a.files, unpushed: a.unpushed, since: e.session.phaseAt });
  }
  clean.sort((a, b) => todoOldest(a, b) || a.title.localeCompare(b.title));
  return { clean, quick, read, count: clean.length + quick.length + read.length };
}

/* The boards to read now: those of the gates listed, read for the first time, or longer ago than `freshMs`, or before a listed gate
   they do not hold was first seen (that read may simply predate it); all of them when `force`. `boards[slug]` is `{ data, at }`,
   `seen[key]` when the page first saw the gate listed (ms), `have(slug, key)` whether the board's data lists the gate. */
function todoSlugsToRead(items, { boards, seen, have, now, freshMs, force = false }) {
  const slugs = new Set();
  for (const w of [...items.quick, ...items.read]) {
    const b = boards[w.board];
    if (force || !b || now - b.at >= freshMs || (b.data && !have(w.board, w.key) && b.at <= (seen[w.key] ?? 0))) slugs.add(w.board);
  }
  return slugs;
}

/* A gate is gone when a read of its board that came after the gate was first seen listed does not hold it: an older read says nothing. */
const todoIsGone = (board, listedAt, held) => !!board?.data && !held && board.at > (listedAt ?? 0);

/* The keys of what is read on the right, in the order the list shows them. */
const todoSelectable = items => items.read.map(w => w.key);

/* What is selected after the list changed: the same one while it is still there; else the one that has taken its place (the next, or
   the last when it was the last); with nothing selected before, the first. */
function todoPickSelection(sel, before, after) {
  if (sel && after.includes(sel)) return sel;
  const at = sel ? before.indexOf(sel) : -1;
  if (at < 0) return after[0] ?? null;
  return after[Math.min(at, after.length - 1)] ?? null;
}

const todoWaited = (since, now) => (Number.isFinite(since) ? `${agoLabel(minutesSince(since, now))}から待ち` : '待ち');

/* What the work document says about whether the list can be trusted: an error with no data is the error and no list (`blocked`); an
   error with old data keeps the data and adds a line. `blocked` is the one line to show instead of the list, `notes` the lines over
   it, and `emptyOk` whether nothing waiting may be said. */
function todoNotes({ workDoc, workError }) {
  if (!workDoc) return { blocked: workError ? `読み込めませんでした: ${workError}` : '読み込み中…', notes: [], emptyOk: false };
  const notes = [];
  let emptyOk = true;
  if (workError) notes.push(`最新を読めませんでした: ${workError}`);
  // A repository that could not be read contributes nothing, so nothing waiting is not known.
  for (const r of (workDoc.repos || []).filter(x => x.error)) { notes.push(`${r.nwo} を読めませんでした: ${r.error}`); emptyOk = false; }
  return { blocked: null, notes, emptyOk };
}

/* A quick question as its row, with where it is from. Until the board it is on has been read there is no question to show, and no button. */
function todoQuickRowHtml(item, g, ctx = {}, now = Date.now() / 1000, error = null, gone = false) {
  const from = `<div class="todo-from">${esc(item.nwo)} · ${esc(todoWaited(item.since, now))}</div>`;
  // Read, and not listed: no question and no button; the next read of the work document drops the row.
  if (!g && gone) return `<div class="todo-quick todo-pending" data-todo-key="${esc(item.key)}"><div class="qc-quick"><b class="qc-quick-text">${esc(item.title)}</b><span class="todo-loading">もう待っていません</span></div>${from}</div>`;
  if (!g) return `<div class="todo-quick todo-pending" data-todo-key="${esc(item.key)}"><div class="qc-quick"><b class="qc-quick-text">${esc(item.title)}</b><span class="todo-loading">${error ? esc(`このボードを読めませんでした: ${error}`) : '読み込み中…'}</span></div>${from}</div>`;
  // The cached question stays (the server refuses an answer to one that no longer waits), with why it may be old.
  const stale = error ? `<div class="todo-note">${esc(`このボードの最新を読めませんでした: ${error}`)}</div>` : '';
  return `<div class="todo-quick" data-todo-key="${esc(item.key)}">${quickRowHtml(g, { ...ctx, key: item.key, firstLine: true })}<div class="todo-task">${esc(item.title)}</div>${stale}${from}</div>`;
}

/* A merged session asked about, as a row of its own: the question and its two buttons, and the repository it is in. Not a gate: it has no
   board to wait for. */
function todoCleanRowHtml(item, busy = false) {
  const ask = { ask: 'clean' };
  return `<div class="todo-quick todo-clean" data-todo-key="${esc(item.key)}">${workMergedAskHtml(ask, { board: item.board, sid: item.sessionId, text: item.text, busy })}`
    + `<div class="todo-from">${esc(item.nwo)}</div></div>`;
}

/* A gate to read as a button that selects it: what kind, which task, what is asked (the worker's own words, else the kind's fixed question). */
function todoReadRowHtml(item, g, sel, now = Date.now() / 1000, gone = false) {
  const [label, color] = kindOf(item.kind);
  const ask = gone && !g ? 'もう待っていません' : gateFocusLine(g?.focus) || QUESTION_ASK_OF_KIND[item.kind] || QUESTION_ASK_OF_KIND.question;
  return `<button type="button" class="todo-row" data-todo-sel="${esc(item.key)}"${sel === item.key ? ' aria-current="true"' : ''}>`
    + `<span class="todo-kind"><i style="background:${esc(color)}"></i>${esc(label)}</span>`
    + `<span class="todo-title">${esc(item.title)}</span><span class="todo-ask">${esc(ask)}</span>`
    + `<span class="todo-from">${esc(item.nwo)} · ${esc(todoWaited(item.since, now))}</span></button>`;
}

/* The whole list: the merged sessions asked about and the quick rows (one tap settles them), then (only when there is something to read) a
   separator and the gates to read. `gateOf(key)` is the gate once its board has been read, `errorOf(key)` why that read failed, `goneOf(key)` whether the board was read and does not list it, `ctxOf(g)` the card's context, `sel` the selected key, `busyOf(key)` whether a request of a merged session's buttons is out. */
function todoListHtml(items, { gateOf = () => null, errorOf = () => null, goneOf = () => false, ctxOf = () => ({}), busyOf = () => false, sel = null, now = Date.now() / 1000, emptyOk = true } = {}) {
  // Nothing waiting is only said when the work document was read whole.
  if (!items.count) return emptyOk ? '<p class="todo-empty">あなたの番はありません</p>' : '';
  const clean = (items.clean || []).map(c => todoCleanRowHtml(c, busyOf(c.key))).join('');
  const quick = items.quick.map(w => { const g = gateOf(w.key); return todoQuickRowHtml(w, g, g ? ctxOf(g) : {}, now, errorOf(w.key), goneOf(w.key)); }).join('');
  const heavy = items.read.length > 0;
  const oneTap = (items.clean || []).length + items.quick.length > 0;
  return clean + quick
    + (heavy && oneTap ? '<h3 class="todo-sep">読んで決める</h3>' : heavy ? '<h3 class="todo-sep first">読んで決める</h3>' : '')
    + items.read.map(w => todoReadRowHtml(w, gateOf(w.key), sel, now, goneOf(w.key))).join('');
}
