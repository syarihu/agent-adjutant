/* ── A PR waiting on your review, in the panel and in the middle ─────────────────────────────────────────────
   Selecting a row of 「AI 下読み」 opens the PR as a task opens in 「いまの仕事」: its read-through in the task panel on the
   right (下読み, 差分, 下書き), the reader's terminal in the middle, 「拡大」 for the whole. It is `selectedTaskId` =
   `pr:<owner/repo#N>` on board `all`, since a PR belongs to no board. What the panel says is pr-read.js's (pure); this file
   holds what the page keeps for it: the stored read-through of the selected PR, the drafts the person is writing and their
   saving, and the terminal. Loaded after my-work-others-view.js, whose records (`others`) it reads. */

/* What pr-diff-view.js sets (the annotated diff): `tabHtml(rec, read)` the 差分 tab; `dialogFileHtml(rec, read, path)`,
   `dialogPaths(rec, read)`, `dialogHeadHtml(rec, read)` and `dialogLabel(rec, path)` the cards of 「拡大」 (keys `diff:<path>` and
   `diff-bar`); `goto(path, line, { inDialog, opener })`; `onShown()` and `onLeft()`
   for the panel's tab (left by another tab, another item or a close); `onMoved()` when the person scrolls 「拡大」 by hand; `onDrawn()` after 「拡大」 replaced its content and `applyTarget()` after it is in place; `act(e, { inDialog })`
   for a click of the diff's own buttons. While it is null the 差分 tab is disabled and 「差分で見る」 is not drawn. */
const PR_HOOKS = { diff: null };
/* The drafts are saved a moment after the last key. */
const PR_SAVE_MS = 800;

const prView = {
  reads: new Map(),   // id -> { key (what was fetched), read (undefined while the first one is on its way, null for none), error, busy }
  drafts: new Map(),  // id -> see prDraftsEntry
  shownFor: null,     // the id the panel was last drawn for: another one starts at the top and reloads its drafts
  focusNext: null,    // { sel, inDialog }: the box an action wants focused once the forced redraw is made
  tabWas: null,       // the saved list tab, while a link has shown 「AI 下読み」 over it
  force: false,       // a redraw that must not wait for a box that has focus (a button was pressed: the boxes may be gone)
  held: false,        // a redraw of the panel's drafts that came while a box of them has focus
};

const prSelected = () => isPrRef(selectedTaskId) ? othersById(prIdOfRef(selectedTaskId)) : null;
/* The address names a PR that can be drawn: the records are in (or could not be read, which the panel says). */
const prReady = () => !!others.doc || others.error != null;
/* What savePrefs writes: the person's own tab, unless they have chosen one since the link. */
function prPrefsToSave() {
  if (prView.tabWas == null) return prefs;
  if (prefs.workTab !== 'others') { prView.tabWas = null; return prefs; }
  return { ...prefs, workTab: prView.tabWas };
}
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
/* The panel is on something else (or closed): the next PR it shows, even this one again, is a new selection. */
function prLeft() {
  prView.shownFor = null;
  // The 差分 tab was left too: its reading place is kept, and what comes of the panel's own reset of the scroll is not it.
  PR_HOOKS.diff?.onLeft?.();
}
/* A deliberate choice of 「AI 下読み」 ends the override a link made of the saved tab; true when there was one. */
function prKeepTab() {
  const was = prView.tabWas != null;
  prView.tabWas = null;
  return was;
}
/* Draw the panel again if it is on a PR (a request of the page changed what its buttons say). */
const prRedraw = () => { if (isPrRef(selectedTaskId)) renderTaskPanel(); };

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
    // Applied to the cache of its own PR whatever is selected now; only the selected one is drawn.
    if (selectedTaskId === PR_REF + rec.id) renderTaskPanel();
  });
}

/* ── the drafts ── */

/* What the page holds of one PR's drafts. `doc` is the file's document (null while there is no file: the reader's drafts then
   stand in, `prSeedDrafts`); `loaded` is that the file has been asked for (editing waits for it, or a save could replace what
   is stored); `dirty` that `doc` has something the server does not; `rev` counts edits, so that a save knows whether more came;
   `inflight` is the promise of the save on its way; `pending` that a push into the pending review is on its way, `pendResult` how the last one went. */
function prDraftsEntry(id) {
  let e = prView.drafts.get(id);
  if (!e) prView.drafts.set(id, e = { doc: null, saved: false, loaded: false, loading: false, error: null, dirty: false, saving: false, saveError: null, timer: 0, rev: 0, inflight: null, pending: false, pendResult: null });
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
    if (selectedTaskId === PR_REF + id) renderTaskPanel();
  });
}
/* What a save sends: only the keys this page knows. The server refuses others, and keeps the ones it has stored itself. */
const PR_DRAFT_KEYS = ['key', 'source', 'path', 'line', 'startLine', 'side', 'body', 'sentAt'];
function prSaveBody(doc) {
  return {
    version: doc.version ?? 1, fromSha: doc.fromSha ?? null,
    drafts: (doc.drafts || []).map(d => Object.fromEntries(PR_DRAFT_KEYS.filter(k => d[k] !== undefined && d[k] !== null).map(k => [k, d[k]]))),
    overall: doc.overall ? { body: doc.overall.body ?? '' } : null,
  };
}
/* The document the page shows: the file's, or the reader's drafts until the first edit makes the file. */
const prDoc = (id, read) => prDraftsEntry(id).doc || prSeedDrafts(read);

/* Change the drafts of `id`: `fn` edits a copy, which is kept and saved. False when they cannot be edited yet, or while they are
   being put into the pending review (what is sent is what was saved). Redrawing is the caller's (typing must not redraw the box
   being typed in). */
function prEditDrafts(id, read, fn) {
  const e = prDraftsEntry(id);
  if (!e.loaded || e.pending) return false;
  const doc = structuredClone(prDoc(id, read));
  fn(doc);
  e.doc = doc;
  e.dirty = true;
  e.rev++;
  // What the last push said (a draft it refused) is about the drafts as they were.
  e.pendResult = null;
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
  for (const root of [tp('task-panel'), cardDialogEl()]) {
    if (!root || selectedTaskId !== PR_REF + id) continue;
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

/* The drafts as a body: the panel's 下書き tab and the dialog's column draw the same, `place` being the only difference. */
function prDraftsBodyHtml(rec, read, place) {
  const e = prDraftsEntry(rec.id);
  // `read` is undefined while the read-through is on its way: drafts seeded from it would be the wrong ones.
  if (!e.loaded || read === undefined) {
    return e.error ? `<div class="wk-notice" role="alert">${esc(`保存済みの下書きを読めませんでした（読み直しています）: ${e.error}`)}</div>`
      : '<div class="tp-muted" role="status">下書きを読み込み中…</div>';
  }
  const doc = prDoc(rec.id, read);
  // After a re-read the person's drafts stay; the reader's new ones are offered.
  const notice = e.saved && read && doc.fromSha !== read.sha ? prNewReaderDrafts(doc, read).length : 0;
  return prDraftsHtml(doc, read, { place, notice, save: prSaveState(e), pend: { sending: e.pending, result: e.pendResult, prUrl: rec.url } });
}

/* What a draft box being focused holds the redraw for. */
const prBoxFocused = root => !!document.activeElement?.matches?.(PR_BOX_SEL) && !!root?.contains(document.activeElement);

/* ── the panel ── */

function renderPrPanel() {
  const panel = tp('task-panel');
  if (!panel) return;
  const id = prIdOfRef(selectedTaskId);
  const rec = othersById(id);
  const records = othersRecords();
  const nowSecs = othersNowEst();
  const cls = document.body.classList;
  panel.hidden = false;
  tp('tp-scrim').hidden = true;
  cls.add('panel-open', 'panel-right');
  cls.remove('panel-pop');
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
  applyRailMode();
  panelShown = null;
  tp('tp-term').hidden = true;
  tp('tp-detail').hidden = false;
  tp('tp-tabs').hidden = false;
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), '');
  renderWorkAway(null);
  renderHandForm(null);
  tp('tp-form').hidden = true;

  // A selection that is new: the list it came from shows, its drafts are read again (unless something here is unsaved), and it starts at the top.
  if (prView.shownFor !== id) {
    prView.shownFor = id;
    const e = prDraftsEntry(id);
    if (!e.dirty && !e.saving) { e.loaded = false; e.error = null; }
    if (!e.pending) e.pendResult = null;
    // The list the link came from shows, for this visit: the tab the person saved is not changed (prPrefsToSave).
    if (prefs.workTab !== 'others') { prView.tabWas = prefs.workTab; prefs.workTab = 'others'; renderWorkTabs(); renderOthersList(); }
    panelScrolledFor = null;
    if (rec) { prEnsureRead(rec); prEnsureDrafts(id); }
  }

  if (!rec) {
    setPanelPart('head', tp('tp-head'), `<div class="tp-head-main"><h2 class="tp-title">この PR</h2></div>${panelBtnsHtml('')}`);
    setPanelPart('tabs', tp('tp-tabs'), '');
    const text = prMissingText(others.doc, others.error);
    setPanelPart('rest', tp('tp-rest'), `<div class="m3-filled-card"><div class="tp-muted" role="status">${esc(text)}</div></div>`);
    return;
  }

  const hubView = othersHubView(others.doc?.hub, records);
  const read = prReadOf(rec);
  const hasRead = prHasRead(rec, read);
  const pane = prPaneOf(nav.pane, hasRead, !!PR_HOOKS.diff);
  const entry = prDraftsEntry(id);
  const draftCount = entry.loaded && hasRead ? prDoc(id, read).drafts.length : 0;
  setPanelPart('head', tp('tp-head'), prHeadHtml(rec));
  setPanelPart('tabs', tp('tp-tabs'), prTabsHtml(rec, read, pane, draftCount, !!PR_HOOKS.diff));
  const opts = { diff: !!PR_HOOKS.diff, asking: others.asking.has(id), readError: prReadEntry(id)?.error || null };
  const html = pane === 'drafts' ? prDraftsBodyHtml(rec, read, 'panel')
    : pane === 'diff' && PR_HOOKS.diff ? PR_HOOKS.diff.tabHtml(rec, read)
    : prDetailHtml(rec, read, records, hubView, nowSecs, opts);
  const key = `${id}/${pane}`;
  // What is typed in a box is kept: the redraw waits until the box is left (prHeldFlush).
  if (prBoxFocused(panel) && panelSig.paneKey === key && !prView.force) prView.held = true;
  else {
    prView.held = false;
    const keep = prFocusKeep(tp('tp-rest'));
    const scroll = tp('tp-detail').scrollTop;
    setPanelPart('rest', tp('tp-rest'), html);
    panelSig.paneKey = key;
    prRestoreFocus(tp('tp-rest'), keep, false);
    if (panelScrolledFor !== key) {
      panelScrolledFor = key;
      tp('tp-detail').scrollTop = 0;
    } else tp('tp-detail').scrollTop = scroll;
    if (pane === 'diff') PR_HOOKS.diff?.onShown?.(); else PR_HOOKS.diff?.onLeft?.();
  }
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
  const id = isPrRef(selectedTaskId) ? prIdOfRef(selectedTaskId) : null;
  const rec = id && othersById(id);
  // Typed into the reader's terminal if that is what the middle holds; what the reader wrote is not typed as it came, since a
  // control character in it would be a key.
  const text = `${prDraftLocation(point)} の「${point.title}」について: `.replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ');
  if (rec && workTerm.term && workTerm.ended == null && workTerm.taskId?.startsWith(`${PR_REF}${rec.id}/`)) {
    if (workTerm.term.type(text)) workTerm.term.focus();
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
    // Saves in flight and edits since: until nothing is unsaved, or one fails.
    for (let n = 0; n < 20 && (e.dirty || e.saving); n++) {
      if (e.saving) await e.inflight; else await prSave(id);
      if (e.saveError != null) throw new Error(`下書きを保存できなかったので、積んでいません: ${e.saveError}`);
    }
    if (e.dirty || e.saving) throw new Error('下書きの保存が終わらなかったので、積んでいません');
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
    if (selectedTaskId === PR_REF + id) prRedrawNow();
  }
}

/* The click of a button that belongs to a PR, in the panel or in 「拡大」. True when it was one. */
function prAct(e, { inDialog = false } = {}) {
  const hit = sel => e.target.closest(sel);
  if (!isPrRef(selectedTaskId)) return false;
  // A click in a dialog that is not about the PR the panel is on belongs to nothing here.
  if (inDialog && cardDialog.subject !== selectedTaskId) return false;
  const id = prIdOfRef(selectedTaskId);
  const rec = othersById(id);
  if (!rec) return false;
  const read = prReadOf(rec) || null;
  const doc = () => prDoc(id, read);
  const boxSel = key => `[data-pr-body="${CSS.escape(key)}"]`;
  // The panel shows what an edit changed; the dialog's column is redrawn by the same call, which follows the panel.
  const edited = (fn, sel = null) => {
    if (prEditDrafts(id, read, fn)) prRedrawNow(sel && { sel, inDialog });
    else if (prDraftsEntry(id).pending) note('下書き', true, 'pending review に積んでいるあいだは、下書きを変えられません');
  };
  let b;
  if ((b = hit('[data-pr-ai], [data-pr-reread]'))) {
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') othersRequestAi(id);
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
    edited(d => {
      if (take && read) d.drafts.push(...prNewReaderDrafts(d, read));
      d.fromSha = read?.sha ?? d.fromSha ?? null;
    }, fresh ? boxSel(fresh.key) : null);
    return true;
  }
  if ((b = hit('[data-pr-goto-diff]'))) {
    const point = read?.points?.[Number(b.dataset.prGotoDiff)];
    if (point) PR_HOOKS.diff?.goto(point.path, point.line, { inDialog, opener: b });
    return true;
  }
  if ((b = hit('[data-pr-goto-file], [data-pr-goto-deleted]'))) {
    const item = b.dataset.prGotoFile != null ? read?.order?.[Number(b.dataset.prGotoFile)] : read?.deleted?.[Number(b.dataset.prGotoDeleted)];
    if (item) PR_HOOKS.diff?.goto(item.path, null, { inDialog, opener: b });
    return true;
  }
  return PR_HOOKS.diff?.act?.(e, { inDialog }) || false;
}

/* The panel's own clicks: its tabs, 「拡大」, close, and what PRs share with the dialog. */
function prPanelClick(e) {
  let b;
  if (e.target.closest('[data-tp-close]')) return closeTaskPanel();
  if ((b = e.target.closest('[data-pr-pane]'))) {
    if (!b.disabled) go({ pane: b.dataset.prPane }, { replace: true });
    return;
  }
  if ((b = e.target.closest('[data-pr-expand]'))) return openPrDialog(b);
  prAct(e);
}

// Typing into a draft changes the document and schedules a save; nothing is drawn, so the box keeps its caret and its composition.
function prInput(e) {
  const t = e.target;
  if (!t.matches?.(PR_BOX_SEL) || !isPrRef(selectedTaskId)) return;
  if (t.closest?.('#card-dialog') && cardDialog.subject !== selectedTaskId) return;
  const id = prIdOfRef(selectedTaskId);
  const rec = othersById(id);
  if (!rec) return;
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
  if (!e.target.matches?.(PR_BOX_SEL) || !isPrRef(selectedTaskId)) return;
  prScheduleSave(prIdOfRef(selectedTaskId), true);
  prHeldFlush(root, e.relatedTarget, held, again);
}
tp('task-panel').addEventListener('input', e => prInput(e));
tp('task-panel').addEventListener('focusout', e => prFocusOut(e, tp('task-panel'), () => prView.held, () => renderTaskPanel()));

/* ── the middle: the reader's terminal ── */


/* The middle for a PR. It uses the work view's terminal slot, but not `syncTermSlot`, which needs a session of a board: the
   socket is the reader's (`/api/others/terminal`), resolved on the server from the record. The slot is kept while the record
   still names the same reader and disposed when it names another (a re-read) or the PR changes. */
function renderPrTerm(rec) {
  if (view !== 'work') return;
  const head = rec ? `<div class="wk-term-title">${esc(rec.title)}</div><div class="wk-term-sub">${esc(prSubText(rec))} · AI 下読みの worker</div>` : '';
  setWorkTermPart('head', wk('wk-term-head'), head);
  const subject = rec ? `${PR_REF}${rec.id}/${rec.ai?.sessionId || ''}` : null;
  const mountable = prReaderOpen(rec);
  if (workTerm.taskId !== subject || (workTerm.reconnect && mountable)) disposeTermSlot(workTerm);
  workTerm.taskId = subject;
  workTerm.board = null;
  if (mountable && !workTerm.term) {
    workTerm.sessionId = rec.ai.sessionId || subject;
    workTerm.ended = null;
    const handle = mountSessionTerminal(workTerm.host(), {
      path: `/api/others/terminal?id=${encodeURIComponent(rec.id)}`,
      base: '',
      focus: false,
      onEnd: code => {
        if (workTerm.term !== handle) return;
        workTerm.ended = code;
        workTerm.redraw();
      },
    });
    workTerm.term = handle;
  }
  setWorkTermPart('bar', wk('wk-term-bar'), '');
  const ph = wk('wk-term-ph');
  // Over a terminal that has ended, so the screen of a dead window is not taken for a live one.
  ph.hidden = !!workTerm.term && workTerm.ended == null;
  const hubView = othersHubView(others.doc?.hub, othersRecords());
  const text = !rec ? `<div class="tp-muted">${esc(prMissingText(others.doc, others.error))}</div>`
    : workTerm.term && workTerm.ended == null ? ''
    : workTerm.term ? prMiddleHtml(rec, othersRecords(), hubView, othersNowEst(), { ended: workTerm.ended, asking: others.asking.has(rec.id) })
    : prMiddleHtml(rec, othersRecords(), hubView, othersNowEst(), { asking: others.asking.has(rec.id) });
  setWorkTermPart('ph', ph, text);
}

wk('wk-term-ph').addEventListener('click', e => {
  if (!isPrRef(nav.task)) return;
  const id = prIdOfRef(nav.task);
  let b;
  if ((b = e.target.closest('[data-pr-ai]'))) {
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') othersRequestAi(id);
    return;
  }
  if (e.target.closest('[data-pr-reconnect]')) {
    workTerm.reconnect = true;
    renderWorkTerm();
  }
});

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
