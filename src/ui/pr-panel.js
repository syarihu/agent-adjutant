/* ── A PR waiting on your review, in the middle and in the panel ─────────────────────────────────────────────
   Selecting a row of 「レビュー」 opens the PR: its read-through in the middle (the groups and the annotated diff, in one scroll, with the person's
   comments on the lines), the reader's terminal in the panel on the right, 「拡大」 for the whole, and 「レビューを送る」 in the heading,
   which opens the dialog that sends the comments (pr-send.js). (A task's own PR is read in a dialog, task-read-view.js.)
   It is `selectedTaskId` = `pr:<owner/repo#N>` on board `all`, since a PR belongs to no board. What it says is pr-read.js's (pure); this
   file holds what the page keeps for it: the stored read-through of the selected PR, the drafts the person is writing and their
   saving, the editors open for new comments, and the terminal. Loaded after my-work-others-view.js, whose records (`others`) it reads. */

/* What pr-diff-view.js sets (the annotated diff): `dialogFileHtml(rec, read, path)`, `dialogPaths(rec, read)`,
   `dialogHeadHtml(rec, read)` and `dialogLabel(rec, path)` the cards of the diff (keys `diff:<path>` and `diff-bar`), in 「拡大」 and in
   the middle; `goto(path, line, { place })` (`place`: 'dialog' or 'read'); `onMoved()` when the person scrolls 「拡大」 by hand; `onDrawn()` after 「拡大」 replaced its content and `applyTarget()` after it is in place; `act(e, { place })`
   for a click of the diff's own buttons; `comments(id)` after the drafts of a PR changed, to bring the comments on the lines up to date; `entries(id)` the files
   of the diff for the send dialog (null: none to show, undefined: still on its way); `input(e)` for what is typed in a comment's box; `pointComment(point, place)` to open a comment
   on a point of a read-through of the older form. While it is null the diff is not drawn and 「差分で見る」 is not offered. */
const PR_HOOKS = { diff: null };
/* The drafts are saved a moment after the last key. */
const PR_SAVE_MS = 800;

const prView = {
  reads: new Map(),   // id -> { key (what was fetched), read (undefined while the first one is on its way, null for none), error, busy }
  drafts: new Map(),  // id -> see prDraftsEntry
  boxes: new Map(),   // id -> Map(key -> the editor being typed in: { kind: 'new', path, side, line, source, text } or { kind: 'edit', text }), in memory only
  shownFor: null,     // the id the middle was last drawn for: another one starts at the top and reloads its drafts
  last: null,         // the id of the PR shown last, so that 「レビュー」 can open it again (in memory only)
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
/* The page is on something else (or closed): the next PR it shows, even this one again, is a new selection. */
function prLeft() {
  prView.shownFor = null;
}
/* Draw the panel again if it is on a PR (a request of the page changed what its buttons say). */
const prRedraw = () => { if (isPrRef(selectedTaskId)) renderTaskPanel(); else if (typeof taskReadRedraw === 'function') taskReadRedraw(); };

/* The drafts of the PR `id` changed, or are being sent: what shows them is drawn again. The heading counts the comments a push would send, the
   comments on the lines are brought up to date in place (an editor being typed in is not replaced), and so is the send dialog if it is open. */
function prDraftsChanged(id) {
  prRedrawFor(id);
  PR_HOOKS.diff?.comments?.(id);
  prSendSync(id);
}

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

/* What the page holds of one PR's drafts. `doc` is the file's document (null while there is no file: an empty one then
   stands in, `prSeedDrafts`); `loaded` is that the file has been asked for (editing waits for it, or a save could replace what
   is stored); `dirty` that `doc` has something the server does not; `rev` counts edits, so that a save knows whether more came;
   `inflight` is the promise of the save on its way; `pending` that a push into the pending review is on its way, `pendResult` how the last one went; `handing` and `handResult` are the same for
   handing a task's drafts to its worker (task-read-drafts.js). */
function prDraftsEntry(id) {
  let e = prView.drafts.get(id);
  if (!e) prView.drafts.set(id, e = { doc: null, saved: false, loaded: false, loading: false, error: null, dirty: false, saving: false, saveError: null, timer: 0, rev: 0, inflight: null, pending: false, pendResult: null, handing: false, handResult: null });
  return e;
}
/* The stored document without what the server adds, with the head each comment was written on (the top-level `shas`, by key) put back on it as `sha`. */
function prDocOfStored(stored) {
  const { id, savedAt, shas, ...doc } = stored || {};
  if (Array.isArray(doc.drafts)) doc.drafts = doc.drafts.map(d => (shas?.[d.key] ? { ...d, sha: shas[d.key] } : d));
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
    prDraftsChanged(id);
  });
}
/* What a save sends: only the keys this page knows. The server refuses others, and keeps the ones it has stored itself. */
const PR_DRAFT_KEYS = ['key', 'source', 'path', 'line', 'startLine', 'side', 'body', 'sentAt'];
function prSaveBody(doc) {
  // The head a comment was written on goes as a top-level map, not as a field of the draft: an older binary refuses unknown fields of a draft.
  const shas = Object.fromEntries((doc.drafts || []).filter(d => d.sha).map(d => [d.key, d.sha]));
  return {
    version: doc.version ?? 1, fromSha: doc.fromSha ?? null, ...(doc.fromMode ? { fromMode: doc.fromMode } : {}),
    drafts: (doc.drafts || []).map(d => Object.fromEntries(PR_DRAFT_KEYS.filter(k => d[k] !== undefined && d[k] !== null).map(k => [k, d[k]]))),
    ...(Object.keys(shas).length ? { shas } : {}),
    overall: doc.overall ? { body: doc.overall.body ?? '' } : null,
  };
}
/* The document the page shows: the file's, or an empty one until the first edit makes the file. */
const prDoc = (id, read) => prDraftsEntry(id).doc || prSeedDrafts(read);

/* Change the drafts of `id`: `fn` edits a copy, which is kept and saved. False when they cannot be edited yet, or while they are
   being put into the pending review or handed to a worker (what is sent is what was saved). Drawing is the caller's (`prDraftsChanged`; typing must not
   draw the box being typed in). */
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

/* Say why the drafts of `id` cannot be changed now, after `prEditDrafts` said no. */
function prEditRefused(id) {
  const e = prDraftsEntry(id);
  if (e.pending) note('コメント', true, 'pending review に積んでいるあいだは、コメントを変えられません');
  else if (e.handing) note('コメント', true, 'worker に渡しているあいだは、コメントを変えられません');
  else note('コメント', true, 'コメントを読み込んでいます。少し待ってからもう一度押してください');
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
  const failedBefore = e.saveError != null;
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
    // Said once, when it starts to fail: the comments are on the lines, where no save line is.
    if (!failedBefore) note('コメントの保存', true, `${e.saveError} 入力はこのページに残っています`);
  } finally {
    e.saving = false;
    done();
  }
  prShowSave(id);
  // Edited while it was on its way, and that save went through.
  if (e.dirty && !e.saveError) prSave(id);
}
const prSaveState = e => e.saveError != null ? { status: 'error', error: e.saveError } : e.saving || e.dirty ? { status: 'saving' } : e.saved ? { status: 'saved' } : { status: '' };
/* The save line of the send dialog, changed in place: a redraw would take the box being typed in. */
function prShowSave(id) {
  const e = prDraftsEntry(id);
  const st = prSaveState(e);
  const text = st.status === 'error' ? `保存できませんでした: ${st.error} 入力はこのページに残っています` : st.status === 'saving' ? '保存しています…' : st.status === 'saved' ? '保存しました' : '';
  for (const root of [document.getElementById('review-send-dialog')]) {
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

/* Write the overall comment that the send dialog shows into the document when it has none: the reader's own text is shown until the person types
   in it, and what a push or a hand-over sends is what was on screen. */
function prSettleOverall(id, read) {
  const doc = prDoc(id, read);
  const { text } = prOverallShown(doc, read);
  if (!doc.overall && text.trim() !== '') prEditDrafts(id, read, d => { d.overall = { body: text }; });
}

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

/* The middle for a review request: its head, and the read-through in one view (task-read-view.js), with the annotated diff in it and the person's
   comments on the lines. The reader's terminal is the panel's (renderPrPanel). Called
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
  const hasRead = !!rec && prHasRead(rec, read);
  const entry = prDraftsEntry(id);
  // What the heading's 「レビューを送る」 counts: the comments a push would send now, once the drafts are loaded.
  const sendN = hasRead && entry.loaded ? prPendable(prDoc(id, read)).drafts : null;
  setWorkTermPart('head', wk('wk-term-head'), rec ? prReviewHeadHtml(rec, read, hubView, asking, sendN) : '');
  const readEl = wk('task-read');
  readEl.dataset.drafts = 'on';
  // The one view has no tabs.
  showMidTabs('', true);
  if (!rec) return taskReadSay(`<div class="m3-filled-card"><div class="tp-muted" role="status">${esc(prMissingText(others.doc, others.error))}</div></div>`);
  const hook = PR_HOOKS.diff;

  const startHeld = !!document.activeElement?.matches?.('[data-task-read-hub-start]');
  // What stands over the read-through when the records could not be read again: it may be behind.
  const behind = others.doc && others.error ? `<div class="wk-notice" role="alert">${esc(`最新の一覧を読み込めませんでした（前に読んだ状態を表示しています）: ${others.error}`)}</div>` : '';
  const key = `pr:${id}`;
  // Before the first read the groups have nothing to say: a block says that it is under way, or that none was made, with the button.
  if (!hasRead && !othersOutOfReview(rec)) {
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
  // The groups and the files of the diff are the index, as the dialog's index lists them in 「拡大」.
  if (drawReadMiddle(key, groups, band, groups.length >= 2 || groups[0].cards.length >= 2)) keepHubStartFocus(startHeld);
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
    if (e.saveError != null) throw new Error(`コメントを保存できなかったので、${notDone}: ${e.saveError}`);
  }
  if (e.dirty || e.saving) throw new Error(`コメントの保存が終わらなかったので、${notDone}`);
}

/* Put the stored drafts into the person's pending review on GitHub. The server sends what is stored, so what is held here is saved
   first (with the overall comment the dialog shows, when the document has none, and as a file when there is none yet). One push in flight per PR: a
   second click does nothing. A failed one keeps the drafts as they are and says so; a successful one takes the server's document, in
   which what was put in carries its `sentAt`. adj never submits the review: the person does, on GitHub. */
async function prPend(id, read) {
  const e = prDraftsEntry(id);
  if (e.pending || !e.loaded) return;
  prSettleOverall(id, read);
  if (!e.doc) prEditDrafts(id, read, () => {});
  e.pending = true;
  e.pendResult = null;
  prDraftsChanged(id);
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
    if (prShown()?.id === id) prDraftsChanged(id);
  }
}

/* The click of a button that belongs to a PR, in 「拡大」 or a task's AI 下読み dialog (`place` 'dialog') or in the read-through of a review
   request (`place` 'read', the default). True when it was one. What is sent is the send dialog's own (pr-send.js). */
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
  let b;
  if ((b = hit('[data-pr-ai], [data-pr-split], [data-pr-reread]'))) {
    // Only the split button asks for a proposal; the re-read button is never drawn while the PR is out of review.
    const mode = b.matches('[data-pr-split]') ? 'split' : undefined;
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') shown.ask(mode);
    return true;
  }
  if ((b = hit('[data-pr-send]'))) {
    if (!b.disabled) prSendOpen();
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
    // A comment on the point's line of the diff, which opens there.
    const point = read?.points?.[Number(b.dataset.prTodraft)];
    if (point) PR_HOOKS.diff?.pointComment?.(point, place);
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
// 「拡大」, 「読み直す」 and 「レビューを送る」 are at the head of the middle.
wk('wk-term-head').addEventListener('click', e => {
  if (!isPrRef(nav.task)) return;
  const b = e.target.closest('[data-pr-expand]');
  if (b) return openPrDialog(b);
  const send = e.target.closest('[data-pr-send]');
  if (send) { if (!send.disabled) prSendOpen(); return; }
  const again = e.target.closest('[data-pr-reread]');
  if (again && !again.disabled && again.getAttribute('aria-disabled') !== 'true') prShown()?.ask();
});
/* 「下読みを閉じる」: asked once, in the page, since closing stops the reader, which cannot be taken back. */
function prCloseReader() {
  const id = prShown()?.id;
  if (id && prReaderOpen(prRecOf(id)) && !others.closing.has(id)) openReaderCloseDialog(id);
}

// Typing into the overall comment of the send dialog changes the document and schedules a save; nothing is drawn, so the box keeps its caret and its
// composition. (A comment on a line is typed in a box of the diff, which keeps its text in the page until it is added: pr-diff-view.js.)
function prInput(e) {
  const t = e.target;
  // Nothing while a task is selected but its read-through is not on screen: the box is not one of this PR's.
  const shown = prShown();
  if (!t.matches?.(PR_BOX_SEL) || !shown?.rec) return;
  const { id, rec } = shown;
  const read = prReadOf(rec) || null;
  const value = t.value;
  prEditDrafts(id, read, doc => { doc.overall = { ...(doc.overall || {}), body: value }; });
}
// Leaving the box saves at once.
function prFocusOut(e) {
  const shown = prShown();
  if (!e.target.matches?.(PR_BOX_SEL) || !shown) return;
  prScheduleSave(shown.id, true);
}

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
