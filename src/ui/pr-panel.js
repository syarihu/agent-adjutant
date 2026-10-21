/* ── A PR waiting on your review, in the middle and in the panel ─────────────────────────────────────────────
   Selecting a row of 「レビュー」 opens the PR: its read-through in the middle, with the tabs 下読み (the read-through and its annotated diff, in one scroll) and 下書き over it, the
   reader's terminal in the panel on the right, 「拡大」 for the whole. (A task's own PR is read in a dialog, task-read-view.js.)
   It is `selectedTaskId` = `pr:<owner/repo#N>` on board `all`, since a PR belongs to no board. What it says is pr-read.js's (pure); this
   file holds what the page keeps for it: the stored read-through of the selected PR, the drafts the person is writing and their
   saving, and the terminal. Loaded after my-work-others-view.js, whose records (`others`) it reads. */

/* What pr-diff-view.js sets (the annotated diff): `dialogFileHtml(rec, read, path)`, `dialogPaths(rec, read)`,
   `dialogHeadHtml(rec, read)` and `dialogLabel(rec, path)` the cards of the diff (keys `diff:<path>` and `diff-bar`), in 「拡大」 and in
   the middle's 下読み tab; `goto(path, line, { place })` (`place`: 'dialog' or 'read'); `onMoved()` when the person scrolls 「拡大」 by hand; `onDrawn()` after 「拡大」 replaced its content and `applyTarget()` after it is in place; `act(e, { place })`
   for a click of the diff's own buttons. While it is null the diff is not drawn and 「差分で見る」 is not offered. */
const PR_HOOKS = { diff: null };
/* The drafts are saved a moment after the last key. */
const PR_SAVE_MS = 800;

const prView = {
  reads: new Map(),   // id -> { key (what was fetched), read (undefined while the first one is on its way, null for none), error, busy }
  drafts: new Map(),  // id -> see prDraftsEntry
  shownFor: null,     // the id the middle was last drawn for: another one starts at the top and reloads its drafts
  focusNext: null,    // { sel, inDialog }: the box an action wants focused once the forced redraw is made
  last: null,         // the id of the PR shown last, so that 「レビュー」 can open it again (in memory only)
  force: false,       // a redraw that must not wait for a box that has focus (a button was pressed: the boxes may be gone)
  held: false,        // a redraw of the panel's drafts that came while a box of them has focus
};

const prSelected = () => isPrRef(selectedTaskId) ? othersById(prIdOfRef(selectedTaskId)) : null;
/* What the read-through's views are on now, so that one set of buttons serves a review request and a task. A review request (selected
   as `pr:<id>`) has its record in the list of the records and asks for a read through the list's endpoint; it has the read-through in
   the middle and the reader's terminal in the panel (`panelTerm`). A task's (the dialog of its AI 下読み, task-read-view.js) has it in
   the task's own document and asks through the task; it has no terminal. `slot` is where the reader's terminal is mounted, null for
   none; `ask(mode)` asks for a read (`mode` 'split' for a proposal). Null while the page is on neither. */
function prShown() {
  if (isPrRef(selectedTaskId)) {
    const id = prIdOfRef(selectedTaskId);
    return { id, rec: othersById(id), kind: 'pr', hubView: othersHubView(others.doc?.hub, othersRecords()), asking: others.asking.has(id), ask: mode => othersRequestAi(id, mode), slot: panelTerm };
  }
  return typeof taskReadShown === 'function' ? taskReadShown() : null;
}
/* The record of `id`: the one on show when it is that PR's, else the list's. */
function prRecOf(id) {
  const shown = prShown();
  return shown && shown.id === id ? shown.rec : othersById(id);
}
/* Draw what shows the PR `id` again, if it is on show: the middle and the panel for a review request, the task's own for a task. */
function prRedrawFor(id) {
  const shown = prShown();
  if (shown?.id !== id) return;
  if (shown.kind === 'pr') renderTaskPanel(); else taskReadRedraw();
}
/* The address names a PR that can be drawn: the records are in (or could not be read, which the panel says). */
const prReady = () => !!others.doc || others.error != null;
/* A structural change (a draft thrown away or added): drawn now even with a box focused, since some browsers do not move focus
   to a pressed button, and the box left on screen could be one that is gone. The focused box keeps its text and caret. */
function prRedrawNow(focus = null) {
  prView.force = true;
  prView.focusNext = focus;
  try { renderTaskPanel(); } finally { prView.force = false; prView.focusNext = null; }
}
/* What has focus in `root`, as something to find again after it is drawn anew: the control by its `data-pr-*` attribute, and the caret. */
function prFocusKeep(root) {
  const el = document.activeElement;
  if (!el || !root?.contains(el)) return null;
  const sel = prFocusSel(el);
  return sel && { sel, a: el.selectionStart, b: el.selectionEnd };
}
/* After a redraw: the box an action asked for (`prView.focusNext`, for the dialog or for the panel), else the control that had focus, with its caret. */
function prRestoreFocus(root, keep, inDialog) {
  const want = prView.focusNext && !!prView.focusNext.inDialog === inDialog ? prView.focusNext.sel : null;
  const asked = want && root.querySelector(want);
  const el = asked || (keep?.sel && root.querySelector(keep.sel));
  if (!el) return null;
  el.focus({ preventScroll: true });
  if (!asked && keep.a != null) { try { el.setSelectionRange(keep.a, keep.b); } catch {} }
  return el;
}
/* The page is on something else (or closed): the next PR it shows, even this one again, is a new selection. */
function prLeft() {
  prView.shownFor = null;
  prView.held = false;
}
/* Draw the panel again if it is on a PR (a request of the page changed what its buttons say). */
const prRedraw = () => { if (isPrRef(selectedTaskId)) renderTaskPanel(); else if (typeof taskReadRedraw === 'function') taskReadRedraw(); };

/* Open the PR: the address carries it, so that a link opens it and back goes to what was open. */
function prOpen(rec) {
  const ref = PR_REF + rec.id;
  // A PR opened again after it was closed is a new selection: its drafts are read again.
  if (nav.task !== ref) prView.shownFor = null;
  go({ board: 'all', view: 'work', task: ref, pane: 'detail' }, { replace: nav.task === ref });
  if (isPrRef(pendingTask)) applyPendingTask();
}

/* ── the read-through ── */

const prReadEntry = id => prView.reads.get(id);
/* undefined while the first fetch is on its way, null when the PR has none, else the read-through. */
function prReadOf(rec) {
  if (!rec?.ai?.sha) return null;
  const e = prReadEntry(rec.id);
  return e ? e.read : undefined;
}

/* Fetch the stored read-through of the PR when it is not the one held: only the selected PR's, and again only when its head or
   finish time changed (it can be 2 MiB, and the records are read every round). */
function prEnsureRead(rec) {
  if (!rec?.ai?.sha) { prView.reads.delete(rec?.id); return; }
  const key = prReadKey(rec);
  let e = prReadEntry(rec.id);
  if (!e) prView.reads.set(rec.id, e = { key: null, read: undefined, error: null, busy: false });
  if (e.key === key || e.busy) return;
  e.busy = true;
  boardApi('', `/api/others/read?id=${encodeURIComponent(rec.id)}`).then(read => {
    e.read = read; e.key = key; e.error = null;
  }, err => {
    // No read-through is an answer; any other failure keeps what was held, and is asked again by the next round.
    if (err.status === 404) { e.read = null; e.key = key; e.error = null; } else e.error = err.message;
  }).finally(() => {
    e.busy = false;
    // Applied to the cache of its own PR whatever is selected now; only the one on show is drawn.
    prRedrawFor(rec.id);
  });
}

/* ── the drafts ── */

/* What the page holds of one PR's drafts. `doc` is the file's document (null while there is no file: the reader's drafts then
   stand in, `prSeedDrafts`); `loaded` is that the file has been asked for (editing waits for it, or a save could replace what
   is stored); `dirty` that `doc` has something the server does not; `rev` counts edits, so that a save knows whether more came;
   `inflight` is the promise of the save on its way; `pending` that a push into the pending review is on its way, `pendResult` how the last one went; `handing` and `handResult` are the same for
   handing a task's drafts to its worker (task-read-drafts.js). */
function prDraftsEntry(id) {
  let e = prView.drafts.get(id);
  if (!e) prView.drafts.set(id, e = { doc: null, saved: false, loaded: false, loading: false, error: null, dirty: false, saving: false, saveError: null, timer: 0, rev: 0, inflight: null, pending: false, pendResult: null, handing: false, handResult: null });
  return e;
}
/* The stored document without what the server adds. */
function prDocOfStored(stored) {
  const { id, savedAt, ...doc } = stored || {};
  return doc;
}
function prEnsureDrafts(id) {
  const e = prDraftsEntry(id);
  if (e.loaded || e.loading) return;
  e.loading = true;
  boardApi('', `/api/others/drafts?id=${encodeURIComponent(id)}`).then(stored => {
    e.doc = prDocOfStored(stored); e.saved = true; e.loaded = true; e.error = null;
  }, err => {
    if (err.status === 404) { e.doc = null; e.saved = false; e.loaded = true; e.error = null; } else e.error = err.message;
  }).finally(() => {
    e.loading = false;
    prRedrawFor(id);
  });
}
/* What a save sends: only the keys this page knows. The server refuses others, and keeps the ones it has stored itself. */
const PR_DRAFT_KEYS = ['key', 'source', 'path', 'line', 'startLine', 'side', 'body', 'sentAt'];
function prSaveBody(doc) {
  return {
    version: doc.version ?? 1, fromSha: doc.fromSha ?? null, ...(doc.fromMode ? { fromMode: doc.fromMode } : {}),
    drafts: (doc.drafts || []).map(d => Object.fromEntries(PR_DRAFT_KEYS.filter(k => d[k] !== undefined && d[k] !== null).map(k => [k, d[k]]))),
    overall: doc.overall ? { body: doc.overall.body ?? '' } : null,
  };
}
/* The document the page shows: the file's, or the reader's drafts until the first edit makes the file. */
const prDoc = (id, read) => prDraftsEntry(id).doc || prSeedDrafts(read);

/* Change the drafts of `id`: `fn` edits a copy, which is kept and saved. False when they cannot be edited yet, or while they are
   being put into the pending review or handed to a worker (what is sent is what was saved). Redrawing is the caller's (typing must not redraw the box
   being typed in). */
function prEditDrafts(id, read, fn) {
  const e = prDraftsEntry(id);
  if (!e.loaded || e.pending || e.handing) return false;
  const doc = structuredClone(prDoc(id, read));
  fn(doc);
  e.doc = doc;
  e.dirty = true;
  e.rev++;
  // What the last push said (a draft it refused) is about the drafts as they were.
  e.pendResult = null;
  e.handResult = null;
  prScheduleSave(id);
  prShowSave(id);
  return true;
}

function prScheduleSave(id, now = false) {
  const e = prDraftsEntry(id);
  clearTimeout(e.timer);
  e.timer = 0;
  if (now) prSave(id); else e.timer = setTimeout(() => prSave(id), PR_SAVE_MS);
}
/* One save in flight per PR. A failed save keeps the text in the page, in memory and in the boxes, and says so; the next edit or
   the box being left saves again. */
async function prSave(id) {
  const e = prDraftsEntry(id);
  clearTimeout(e.timer);
  e.timer = 0;
  if (!e.dirty || e.saving || !e.doc) return;
  e.saving = true;
  e.saveError = null;
  const rev = e.rev;
  let done;
  e.inflight = new Promise(r => { done = r; });
  prShowSave(id);
  try {
    await boardApi('', '/api/others/drafts', { method: 'POST', body: JSON.stringify({ id, drafts: prSaveBody(e.doc) }) });
    e.saved = true;
    if (e.rev === rev) e.dirty = false;
  } catch (err) {
    const ps = err.problems || [];
    e.saveError = ps.length ? `${err.message} (${ps.slice(0, 3).join(' / ')}${ps.length > 3 ? ` / 他 ${ps.length - 3} 件` : ''})` : err.message;
  } finally {
    e.saving = false;
    done();
  }
  prShowSave(id);
  // Edited while it was on its way, and that save went through.
  if (e.dirty && !e.saveError) prSave(id);
}
const prSaveState = e => e.saveError != null ? { status: 'error', error: e.saveError } : e.saving || e.dirty ? { status: 'saving' } : e.saved ? { status: 'saved' } : { status: '' };
/* The save line of the panel and of the dialog's column, changed in place: a redraw would take the box being typed in. */
function prShowSave(id) {
  const e = prDraftsEntry(id);
  const st = prSaveState(e);
  const text = st.status === 'error' ? `保存できませんでした: ${st.error} 入力はこのページに残っています` : st.status === 'saving' ? '保存しています…' : st.status === 'saved' ? '保存しました' : '';
  for (const root of [tp('task-panel'), wk('task-read'), cardDialogEl()]) {
    if (!root || prShown()?.id !== id) continue;
    root.querySelectorAll('[data-pr-save]').forEach(el => {
      if (el.textContent !== text) el.textContent = text;
      el.classList.toggle('err', st.status === 'error');
    });
  }
}
// A page closed with something unsaved says so: the text exists nowhere else.
window.addEventListener('beforeunload', e => {
  if ([...prView.drafts.values()].some(d => d.dirty)) { e.preventDefault(); e.returnValue = ''; }
});

/* The drafts as a body: the middle's 下書き tab, a task's 「worker への指示」 (in its dialog's column) and the review dialog's column draw the same, `place` being the only difference. `to` is
   where they are going: 'pending' (a review request's: a pending review on GitHub) or 'worker' (a task's: its worker). */
function prDraftsBodyHtml(rec, read, place, to = 'pending') {
  const e = prDraftsEntry(rec.id);
  // `read` is undefined while the read-through is on its way: drafts seeded from it would be the wrong ones.
  if (!e.loaded || read === undefined) {
    return e.error ? `<div class="wk-notice" role="alert">${esc(`保存済みの下書きを読めませんでした（読み直しています）: ${e.error}`)}</div>`
      : '<div class="tp-muted" role="status">下書きを読み込み中…</div>';
  }
  const doc = prDoc(rec.id, read);
  // After a re-read the person's drafts stay; the reader's new ones are offered.
  const notice = e.saved && read && prFromElsewhere(doc, read) ? prNewReaderDrafts(doc, read).length + (prNewReaderOverall(doc, read) ? 1 : 0) : 0;
  const where = to === 'worker' ? { to, hand: { sending: e.handing, result: e.handResult } } : { pend: { sending: e.pending, result: e.pendResult, prUrl: rec.url } };
  return prDraftsHtml(doc, read, { place, notice, save: prSaveState(e), ...where });
}

/* What a draft box being focused holds the redraw for. */
const prBoxFocused = root => !!document.activeElement?.matches?.(PR_BOX_SEL) && !!root?.contains(document.activeElement);

/* ── the middle: the read-through, and the panel: the reader ── */

/* A selection that is new: its drafts are read again (unless something here is unsaved), and it starts at the top. */
function prEnter(id, rec) {
  if (prView.shownFor === id) return;
  prView.shownFor = id;
  prView.last = id;
  const e = prDraftsEntry(id);
  if (!e.dirty && !e.saving) { e.loaded = false; e.error = null; }
  if (!e.pending) e.pendResult = null;
  panelScrolledFor = null;
  if (rec) { prEnsureRead(rec); prEnsureDrafts(id); }
}

/* The middle for a review request: its head, the tabs 下読み and 下書き, and the tab on show drawn in the read-through's place
   (task-read-view.js). The reader's terminal is the panel's (renderPrPanel). Called
   from `renderWorkTerm`, which runs before the panel is drawn. */
function renderPrRead(rec) {
  const id = prIdOfRef(nav.task);
  // A review request has no session of a board: nothing of the middle's own is kept behind the read-through.
  disposeTermSlot(workTerm);
  workTerm.board = null;
  prEnter(id, rec);
  const records = rec ? othersRecords() : [];
  const hubView = rec ? othersHubView(others.doc?.hub, records) : null;
  const read = rec ? prReadOf(rec) : undefined;
  const asking = others.asking.has(id);
  setWorkTermPart('head', wk('wk-term-head'), rec ? prReviewHeadHtml(rec, read, hubView, asking) : '');
  const readEl = wk('task-read');
  readEl.dataset.drafts = 'on';
  if (!rec) {
    showMidTabs('', true);
    // No tabs to name it.
    readEl.removeAttribute('aria-labelledby');
    return taskReadSay(`<div class="m3-filled-card"><div class="tp-muted" role="status">${esc(prMissingText(others.doc, others.error))}</div></div>`);
  }
  const hasRead = prHasRead(rec, read);
  // The diff is part of the 下読み tab; a split proposal did not read it (prDialogGroups leaves it out).
  const hook = PR_HOOKS.diff;
  const pane = prPaneOf(nav.pane, hasRead);
  const entry = prDraftsEntry(id);
  const draftCount = entry.loaded && hasRead ? prDoc(id, read).drafts.length : 0;
  showMidTabs(prTabsHtml(rec, read, pane, draftCount), true);
  readEl.setAttribute('aria-labelledby', `wk-mid-tab-pr-${pane}`);

  const startHeld = !!document.activeElement?.matches?.('[data-task-read-hub-start]');
  // What stands over the read-through when the records could not be read again: it may be behind.
  const behind = others.doc && others.error ? `<div class="wk-notice" role="alert">${esc(`最新の一覧を読み込めませんでした（前に読んだ状態を表示しています）: ${others.error}`)}</div>` : '';
  const key = `pr:${id}/${pane}`;
  if (pane === 'drafts') return prDrawDrafts(key, rec, read, behind);
  // The flag means that the drafts tab is waiting for a box to be left: no other tab waits.
  prView.held = false;
  // Before the first read the groups have nothing to say: a block says that it is under way, or that none was made, with the button.
  if (pane === 'detail' && !hasRead && !othersOutOfReview(rec)) {
    const band = behind + taskReadHubHtml(rec, hubView, others.starting);
    taskReadSay(band + prNoReadHtml(rec, othersAiView(rec, records, othersNowEst()), hubView, asking));
    return keepHubStartFocus(startHeld);
  }
  // A split proposal did not read the diff: none is fetched for it.
  const diffOn = !!hook && !!read && read.mode !== 'split';
  const groups = taskReadGroups(rec, read, hubView, othersNowEst(), {
    records, since: true, diff: !!hook, asking, readError: !!prReadEntry(id)?.error,
    diffFileHtml: diffOn ? path => hook.dialogFileHtml(rec, read, path) : null,
    diffPaths: diffOn ? hook.dialogPaths(rec, read) : null,
    diffHeadHtml: diffOn ? hook.dialogHeadHtml(rec, read) : null,
    diffLabel: diffOn ? path => hook.dialogLabel(rec, path) : null,
  });
  // The read-through is on its way: nothing of it can be drawn yet.
  if (!groups.length) return taskReadSay('<div class="tp-muted" role="status">下読みを読み込み中…</div>');
  const band = behind + taskReadHubHtml(rec, hubView, others.starting) + taskReadBandHtml(rec, read, hubView, asking);
  // The groups and the files of the diff are the index of the tab, as the dialog's index lists them in 「拡大」.
  if (drawReadMiddle(key, groups, band, groups.length >= 2 || groups[0].cards.length >= 2)) keepHubStartFocus(startHeld);
}

/* The 下書き tab: the drafts in the middle's place, one column. What is typed in a box is kept: the redraw waits until the box is left
   (prHeldFlush). */
function prDrawDrafts(key, rec, read, band) {
  const root = wk('task-read');
  const html = prDraftsBodyHtml(rec, read, 'read');
  if (prBoxFocused(root) && taskRead.drawnKey === key && !prView.force) { prView.held = true; return; }
  prView.held = false;
  const content = wk('task-read-content');
  const body = wk('task-read-body');
  // Whatever the other tabs drew is not what this holds: they are drawn anew when they come back.
  if (taskRead.drawnKey !== key) {
    taskRead.drawnKey = key;
    taskRead.drawn.order = '';
    taskRead.drawn.html = new Map();
    taskRead.contentSig = '';
  }
  taskRead.groups = [];
  wk('task-read-index').hidden = true;
  const sig = `${band}\u0000${html}`;
  if (sig !== taskRead.contentSig) {
    taskRead.contentSig = sig;
    const keep = prFocusKeep(content);
    wk('task-read-band').innerHTML = band;
    content.innerHTML = html;
    prRestoreFocus(content, keep, false);
  }
  if (taskRead.shownFor !== key) {
    taskRead.shownFor = key;
    body.scrollTop = taskRead.scroll.get(key) || 0;
  }
}

/* The panel for a review request: its reader's terminal, and nothing else. */
function renderPrPanel() {
  const panel = tp('task-panel');
  if (!panel) return;
  const id = prIdOfRef(selectedTaskId);
  const rec = othersById(id);
  const cls = document.body.classList;
  panel.hidden = false;
  tp('tp-scrim').hidden = true;
  cls.add('panel-open', 'panel-right');
  cls.remove('panel-pop');
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
  panelShown = null;
  panelScrolledFor = null;
  const reveal = tp('tp-term').hidden;
  tp('tp-tabs').hidden = true;
  setPanelPart('tabs', tp('tp-tabs'), '');
  tp('tp-detail').hidden = true;
  setPanelPart('rest', tp('tp-rest'), '');
  panelSig.paneKey = null;
  tp('tp-term').hidden = false;
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), '');
  renderWorkAway(null);
  renderHandForm(null);
  tp('tp-form').hidden = true;

  setPanelPart('head', tp('tp-head'), rec ? taskReaderHeadHtml(rec, others.closing.has(id))
    : `<div class="tp-head-main"><h2 class="tp-title">この PR</h2></div>${panelBtnsHtml('')}`);
  const records = othersRecords();
  drawReaderTerm(rec, records, othersHubView(others.doc?.hub, records), `<div class="tp-muted">${esc(prMissingText(others.doc, others.error))}</div>`);
  if (reveal && panelTerm.term) panelTerm.term.fit();
}

/* A redraw that was held while a box had focus, made once focus is somewhere else. */
function prHeldFlush(root, relatedTarget, held, again) {
  if (!held()) return;
  // Focus moving to a button of the panel: drawing now would replace the button between its press and its click. That click
  // (or the next round) redraws instead.
  if (relatedTarget && root.contains(relatedTarget)) return;
  setTimeout(() => { if (held() && !prBoxFocused(root)) again(); });
}

/* ── what the panel's buttons do (and the dialog's: `inDialog`) ── */

function prAsk(point) {
  const shown = prShown();
  const rec = shown?.rec;
  const slot = shown?.slot;
  // Typed into the reader's terminal if that is where it is mounted (a review request's, in the panel); what the reader wrote is not
  // typed as it came, since a control character in it would be a key.
  const text = `${prDraftLocation(point)} の「${point.title}」について: `.replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ');
  if (rec && slot?.term && slot.ended == null && slot.taskId?.startsWith(`${PR_REF}${rec.id}/`)) {
    if (slot.term.type(text)) slot.term.focus();
    else note('聞く', true, 'AI 下読みの worker のターミナルが開いていません');
  } else note('聞く', true, 'AI 下読みの worker のターミナルが開いていません');
}

async function prCopy(text) {
  if (!String(text || '').trim()) return note('コピー', true, 'コピーする内容がありません');
  try {
    if (!navigator.clipboard?.writeText) throw new Error('no clipboard');
    await navigator.clipboard.writeText(text);
    note('コピー', false, 'コピーしました');
  } catch {
    note('コピー', true, 'このページからはクリップボードに書けません');
  }
}

/* Wait until nothing of `id`'s drafts is unsaved: the saves in flight and the edits since. Throws, saying `notDone` is what was not
   done, when one fails or they do not settle. What the server reads is the file, so whatever sends the drafts waits for this first. */
async function prSaveSettled(id, notDone) {
  const e = prDraftsEntry(id);
  for (let n = 0; n < 20 && (e.dirty || e.saving); n++) {
    if (e.saving) await e.inflight; else await prSave(id);
    if (e.saveError != null) throw new Error(`下書きを保存できなかったので、${notDone}: ${e.saveError}`);
  }
  if (e.dirty || e.saving) throw new Error(`下書きの保存が終わらなかったので、${notDone}`);
}

/* Put the stored drafts into the person's pending review on GitHub. The server sends what is stored, so what is held here is saved
   first (and the reader's drafts, which are not a file until the first edit, are made the file). One push in flight per PR: a
   second click does nothing. A failed one keeps the drafts as they are and says so; a successful one takes the server's document, in
   which what was put in carries its `sentAt`. adj never submits the review: the person does, on GitHub. */
async function prPend(id, read) {
  const e = prDraftsEntry(id);
  if (e.pending || !e.loaded) return;
  if (!e.doc) prEditDrafts(id, read, () => {});
  e.pending = true;
  e.pendResult = null;
  prRedrawNow();
  try {
    await prSaveSettled(id, '積んでいません');
    const res = await boardApi('', '/api/others/pending', { method: 'POST', body: JSON.stringify({ id }) });
    if (res.drafts) { e.doc = prDocOfStored(res.drafts); e.saved = true; e.dirty = false; }
    else {
      // GitHub has them but adj could not write its own file: the drafts are marked here, so that a second click does not send them
      // again, and the next save (which carries `sentAt`) writes the marks.
      const doc = structuredClone(prDoc(id, read));
      const at = new Date().toISOString().replace(/[-:]|\.\d+/g, '');
      for (const d of doc.drafts) if ((res.sent || []).includes(d.key)) d.sentAt = at;
      if (res.overall && res.overall !== 'none') doc.pending = { ...(doc.pending || {}), overall: doc.overall?.body ?? '' };
      e.doc = doc; e.dirty = true; e.rev++;
      prScheduleSave(id);
    }
    e.pendResult = { sent: (res.sent || []).length, failed: res.failed || [], skipped: res.skipped || [], overall: res.overall || 'none' };
    if (res.warning) note('pending review', true, `GitHub には積みましたが、adj の記録に書けませんでした: ${res.warning}`);
    // The row left 新着: the records are read again (the same as after asking for a read). The push has gone through whatever this does.
    try { await refreshOthers(true); } catch {}
  } catch (err) {
    note('pending review', true, err.message);
  } finally {
    e.pending = false;
    if (prShown()?.id === id) prRedrawNow();
  }
}

/* The click of a button that belongs to a PR, in 「拡大」 or a task's AI 下読み dialog (`place` 'dialog') or in the read-through of a review
   request (`place` 'read', the default). True when it was one. */
function prAct(e, { place = 'read' } = {}) {
  const hit = sel => e.target.closest(sel);
  const shown = prShown();
  if (!shown) return false;
  const inDialog = place === 'dialog';
  // A click in a dialog that is not about the PR the panel is on belongs to nothing here.
  if (inDialog && cardDialog.subject !== selectedTaskId) return false;
  const id = shown.id;
  const rec = shown.rec;
  if (!rec) return false;
  const read = prReadOf(rec) || null;
  const doc = () => prDoc(id, read);
  const boxSel = key => `[data-pr-body="${CSS.escape(key)}"]`;
  // The panel shows what an edit changed; the dialog's column is redrawn by the same call, which follows the panel.
  const edited = (fn, sel = null) => {
    if (prEditDrafts(id, read, fn)) prRedrawNow(sel && { sel, inDialog });
    else if (prDraftsEntry(id).pending) note('下書き', true, 'pending review に積んでいるあいだは、下書きを変えられません');
    else if (prDraftsEntry(id).handing) note('下書き', true, 'worker に渡しているあいだは、下書きを変えられません');
  };
  let b;
  if ((b = hit('[data-pr-ai], [data-pr-split], [data-pr-reread]'))) {
    // Only the split button asks for a proposal; the re-read button is never drawn while the PR is out of review.
    const mode = b.matches('[data-pr-split]') ? 'split' : undefined;
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') shown.ask(mode);
    return true;
  }
  if ((b = hit('[data-pr-ask]'))) {
    const point = read?.points?.[Number(b.dataset.prAsk)];
    if (!point) return true;
    // Leaving for the terminal: the dialog goes first, and focus is not given back to the panel.
    if (inDialog) { cardDialog.quiet = true; closeCardDialog(); }
    prAsk(point);
    return true;
  }
  if ((b = hit('[data-pr-todraft]'))) {
    const point = read?.points?.[Number(b.dataset.prTodraft)];
    if (!point) return true;
    if (!prDraftsEntry(id).loaded) {
      note('下書き', true, '下書きを読み込んでいます。少し待ってからもう一度押してください');
      return true;
    }
    // The tab first, so the box that is made is drawn in it; it takes the keyboard, where the person finishes it.
    if (!inDialog && nav.pane !== 'drafts') go({ pane: 'drafts' }, { replace: true });
    const key = prNextKey(doc(), 'p');
    edited(d => { d.drafts.push(prDraftFromPoint(point, d)); }, boxSel(key));
    return true;
  }
  if ((b = hit('[data-pr-pend]'))) {
    if (!b.disabled) prPend(id, read);
    return true;
  }
  if ((b = hit('[data-pr-copy]'))) {
    const d = doc().drafts.find(x => x.key === b.dataset.prCopy);
    if (d) prCopy(`${prDraftLocation(d)}\n${d.body}`);
    return true;
  }
  if (hit('[data-pr-copy-all]')) { prCopy(prCopyAllText(doc())); return true; }
  if ((b = hit('[data-pr-discard]'))) {
    const key = b.dataset.prDiscard;
    // The keyboard goes to the next draft, else the one before, else the overall comment.
    const keys = doc().drafts.map(x => x.key);
    const near = keys[keys.indexOf(key) + 1] ?? keys[keys.indexOf(key) - 1];
    edited(d => { d.drafts = d.drafts.filter(x => x.key !== key); }, near != null && near !== key ? boxSel(near) : '[data-pr-overall]');
    return true;
  }
  if (hit('[data-pr-take-new]') || hit('[data-pr-keep]')) {
    const take = !!hit('[data-pr-take-new]');
    const fresh = take && read ? prNewReaderDrafts(doc(), read)[0] : null;
    const overall = take && read ? prNewReaderOverall(doc(), read) : null;
    edited(d => {
      if (take && read) d.drafts.push(...prNewReaderDrafts(d, read));
      // The reader's overall comment is put after what the person has written, never over it.
      const o = take && read ? prNewReaderOverall(d, read) : null;
      if (o != null) d.overall = { ...(d.overall || {}), body: String(d.overall?.body ?? '').trim() === '' ? o : `${d.overall.body}\n\n${o}` };
      d.fromSha = read?.sha ?? d.fromSha ?? null;
      if (read) { if (read.mode === 'split') d.fromMode = 'split'; else delete d.fromMode; }
    }, fresh ? boxSel(fresh.key) : overall != null ? '[data-pr-overall]' : null);
    return true;
  }
  if ((b = hit('[data-pr-goto-diff]'))) {
    const point = read?.points?.[Number(b.dataset.prGotoDiff)];
    if (point) PR_HOOKS.diff?.goto(point.path, point.line, { place });
    return true;
  }
  if ((b = hit('[data-pr-goto-since]'))) {
    const c = read?.sinceReview?.comments?.[Number(b.dataset.prGotoSince)];
    if (c?.path) PR_HOOKS.diff?.goto(c.path, c.line, { place });
    return true;
  }
  if ((b = hit('[data-pr-goto-file], [data-pr-goto-deleted]'))) {
    const item = b.dataset.prGotoFile != null ? read?.order?.[Number(b.dataset.prGotoFile)] : read?.deleted?.[Number(b.dataset.prGotoDeleted)];
    if (item) PR_HOOKS.diff?.goto(item.path, null, { place });
    return true;
  }
  return PR_HOOKS.diff?.act?.(e, { place }) || false;
}

/* The panel's own clicks: close, 「下読みを閉じる」, 再接続, and what PRs share with the dialog (the buttons of the placeholder over the terminal). */
function prPanelClick(e) {
  if (e.target.closest('[data-tp-close]')) return closeTaskPanel();
  if (e.target.closest('[data-pr-close-reader]')) return prCloseReader();
  if (e.target.closest('[data-pr-reconnect]')) {
    panelTerm.reconnect = true;
    return renderTaskPanel();
  }
  prAct(e);
}
// 「拡大」 and 「読み直す」 are at the head of the middle.
wk('wk-term-head').addEventListener('click', e => {
  if (!isPrRef(nav.task)) return;
  const b = e.target.closest('[data-pr-expand]');
  if (b) return openPrDialog(b);
  const again = e.target.closest('[data-pr-reread]');
  if (again && !again.disabled && again.getAttribute('aria-disabled') !== 'true') prShown()?.ask();
});
/* 「下読みを閉じる」: asked once, in the page, since closing stops the reader, which cannot be taken back. */
function prCloseReader() {
  const id = prShown()?.id;
  if (id && prReaderOpen(prRecOf(id)) && !others.closing.has(id)) openReaderCloseDialog(id);
}

// Typing into a draft changes the document and schedules a save; nothing is drawn, so the box keeps its caret and its composition.
function prInput(e) {
  const t = e.target;
  // Nothing while a task is selected but its read-through is not on screen: the box is not one of this PR's.
  const shown = prShown();
  if (!t.matches?.(PR_BOX_SEL) || !shown?.rec) return;
  if (t.closest?.('#card-dialog') && cardDialog.subject !== selectedTaskId) return;
  const { id, rec } = shown;
  const read = prReadOf(rec) || null;
  const value = t.value;
  // What is in the pending review is not edited here.
  if (!t.matches('.pr-overall') && prDoc(id, read).drafts.find(x => x.key === t.dataset.prBody)?.sentAt) return;
  let readded = null;
  prEditDrafts(id, read, doc => {
    if (t.matches('.pr-overall')) doc.overall = { ...(doc.overall || {}), body: value };
    else {
      const d = doc.drafts.find(x => x.key === t.dataset.prBody);
      if (d) d.body = value;
      else {
        // The box on screen is one whose draft is gone: what is typed is not dropped, it becomes a draft of its own.
        readded = prNextKey(doc, 'p');
        doc.drafts.push({ key: readded, source: 'person', side: 'right', body: value });
      }
    }
  });
  if (readded) {
    note('下書き', true, '消した下書きの箱に入力がありました。新しい下書きとして残しました');
    prRedrawNow({ sel: `[data-pr-body="${CSS.escape(readded)}"]`, inDialog: !!t.closest?.('#card-dialog') });
  }
}
// Leaving a box saves at once, and lets a redraw that waited for it be made.
function prFocusOut(e, root, held, again) {
  const shown = prShown();
  if (!e.target.matches?.(PR_BOX_SEL) || !shown) return;
  prScheduleSave(shown.id, true);
  prHeldFlush(root, e.relatedTarget, held, again);
}
// The task panel's drafts (「worker への指示」) are held the same way; a review request's are in the middle (task-read-view.js).
tp('task-panel').addEventListener('focusout', e => prFocusOut(e, tp('task-panel'), () => prView.held, () => renderTaskPanel()));

/* ── the reader's terminal ── */

/* The subject the reader's terminal is kept under: a closed reader is part of it, so that a re-read does not find the old one. */
const prReaderSubject = rec => rec ? `${PR_REF}${rec.id}/${rec.ai?.sessionId || ''}${rec.ai?.readerClosedAt ? '/closed' : ''}` : null;

/* What a terminal that has ended says to `prMiddleHtml`: `ended` when the worker really has ended (the server says its window is gone,
   or it detached because the window left), else `failed` with why, since an attach that failed is not an end (a worker that runs
   is not said to have ended). */
const PR_TERM_ENDS = [1000, 4410];
/* The words of a failed attach: the page's for the code, and the server's reason after it unless those words are the reason already. */
const prFailedText = (code, reason) => {
  const text = terminalEndText(code, reason);
  return reason && !text.includes(reason) ? `${text}: ${reason}` : text;
};
function prTermEnd(slot) {
  if (!slot.term || slot.ended == null) return {};
  return PR_TERM_ENDS.includes(slot.ended) ? { ended: slot.ended } : { failed: prFailedText(slot.ended, slot.endReason) };
}

/* The reader's terminal in `slot` (the panel's): kept while the record still names
   the same reader, disposed when it names another (a re-read) or the PR changes, and mounted when there is a window to attach to.
   The socket is the reader's (`/api/others/terminal`), resolved on the server from the record. */
function syncReaderTerm(slot, rec, subject, mount = true) {
  const mountable = prReaderOpen(rec);
  if (slot.taskId !== subject || (slot.reconnect && mountable)) disposeTermSlot(slot);
  slot.taskId = subject;
  // `mount` is false while its pane is hidden: a terminal mounted there has no size to fit to. One already mounted is kept.
  // A reconnect asked for when there is nothing to mount must not wait for the next reader.
  if (!mountable) slot.reconnect = false;
  if (!mountable || slot.term || !mount) return;
  slot.sessionId = rec.ai.sessionId || subject;
  slot.ended = null;
  slot.endReason = null;
  const handle = mountSessionTerminal(slot.host(), {
    path: `/api/others/terminal?id=${encodeURIComponent(rec.id)}`,
    base: '',
    focus: false,
    onEnd: (code, reason) => {
      if (slot.term !== handle) return;
      slot.ended = code;
      slot.endReason = reason;
      slot.redraw();
    },
  });
  slot.term = handle;
}

/* ── following the records ── */

/* After every read of the records: a PR named by the address is opened once they are here, and the one that is open follows its
   record (the read-through is fetched when its head or finish time changed). */
function prFollow() {
  if (view !== 'work') return;
  if (isPrRef(pendingTask)) applyPendingTask();
  if (!isPrRef(selectedTaskId)) return;
  const rec = prSelected();
  if (rec) {
    prEnsureRead(rec);
    prEnsureDrafts(rec.id);
  }
  renderTaskPanel();
}
