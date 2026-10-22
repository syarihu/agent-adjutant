/* ── 「AI 下読み」 of a task: its document, and the dialog the terminal's heading opens ───────────────────────────────────────────
   A task whose PR has a read-through (docs/architecture.md, "A task's PR") has 「下読みを見る」 (or 「下読みする」) in the heading of its
   terminal. It opens the read-through in the card dialog (`#card-dialog`, mode 'own-read'): the read-through in one scroll laid out as
   「拡大」 lays out a review request's, with no index and no reader's terminal, the person's comments on the lines of its diff, and 「worker に渡す」 in its heading, which opens the dialog that
   hands them over (pr-send.js). What
   the markup says is task-read.js's (pure) and, below it, pr-read.js's; what is drawn is the review dialog's machinery
   (`prDialogPaint`), and the buttons are the review request's (`prAct`, with `place: 'dialog'`). This file keeps what the page holds
   of it: the task's document, its poll, the dialog, and the middle's drawing for a review request (pr-panel.js uses it). Loaded after
   pr-diff-view.js. */

const taskRead = {
  key: null,          // `<board>/<task>/<pr>`: what `doc` is for; an answer is used only while this is still the task on show
  doc: null,          // `GET /api/tasks/<id>/ai`: { pr, record, hub }, null until it has come
  sig: '',            // `doc` as JSON, so that an answer that says what was said draws nothing
  error: null,        // why the last read failed
  busy: false,        // a read is out
  again: false,       // a forced read asked for while one was out
  headMode: '',       // what the terminal's heading button says as it was drawn (`taskReadHeadMode`), so that a document that changes it draws the heading
  drawn: { order: '', html: new Map() },  // what the middle was last drawn from (`prDialogPatch`)
  contentSig: '',     // the middle's markup as drawn
  drawnKey: null,     // what `drawn` and `contentSig` are of: a review request's `pr:<id>`
  groups: [],         // the groups drawn
  shownFor: null,     // the key the middle is drawn for while it is on screen, null while the terminal is
  draftsFor: null,    // `<key>#<opening>`: the opening of the dialog the PR's drafts were last asked for, asked again at the next
  scroll: new Map(),  // key -> the middle's scroll top
  active: null,       // the group the index marks
  pinned: null,       // the group a jump left, marked until the person scrolls
  pinnedAt: 0,
  spying: false,
};

/* The dialog is open on a task that has a PR: the work view is on it, and the dialog was opened for it (it stays 'own-read' with no subject once closed). */
const taskReadActive = task => !!task && view === 'work' && !!task.pr && task.id === nav.task && cardDialog.mode === 'own-read' && cardDialog.subject === task.id;
const taskReadTaskOf = () => view === 'work' && isTaskRef(nav.task) ? taskById(nav.task) || null : null;
const taskReadTask = () => { const t = taskReadTaskOf(); return taskReadActive(t) ? t : null; };
const taskReadKey = task => `${nav.board}/${task.id}/${task.pr}`;

/* What the read-through's buttons are on, for `prShown` (pr-panel.js): null unless the dialog is open on the task, and until the task's
   document has come, or when its PR is not one that can be read. `rec` is null while the PR has no record. There is no reader's terminal
   (`slot` is null). */
function taskReadShown() {
  const task = taskReadTask();
  if (!task || taskRead.key !== taskReadKey(task) || !taskRead.doc?.pr) return null;
  const { pr: id, record: rec, hub } = taskRead.doc;
  return { id, rec, kind: 'task', hubView: othersHubView(hub, rec ? [rec] : []), asking: others.asking.has(id), ask: mode => taskReadAsk(mode), slot: null };
}

/* ── the document ── */

/* Read the task's document when it has a PR: the heading's button says 「下読みを見る」 or 「下読みする」 by it, so this is asked for on every
   poll while the task is selected (a read in flight is not asked again). A task that is not the one the document is for starts
   it over. */
async function taskReadRefresh(force = false) {
  const task = taskReadTaskOf();
  // An older link to the read-through of a task that has no PR has nothing to open: it must not open one the task gets later.
  if (task && !task.pr && pendingOwnRead === `${nav.board}/${task.id}`) pendingOwnRead = null;
  if (!task?.pr || !multiBoard || !nav.board || nav.board === 'all') return;
  const key = taskReadKey(task);
  if (taskRead.key !== key) {
    Object.assign(taskRead, { key, doc: null, sig: '', error: null, contentSig: '', groups: [], shownFor: null, active: null, pinned: null });
    // What the middle was drawn from is another PR's: cards are reused by their place, so it must not be patched into this one.
    taskRead.drawn.order = '';
    taskRead.drawn.html = new Map();
  }
  if (taskRead.busy) {
    if (force) taskRead.again = true;
    return;
  }
  taskRead.busy = true;
  const epoch = navEpoch;
  let changed = false;
  try {
    const doc = await boardApi(BASE, `/api/tasks/${encodeURIComponent(task.id)}/ai`);
    if (taskRead.key === key && epoch === navEpoch) {
      const sig = JSON.stringify(doc);
      changed = sig !== taskRead.sig || taskRead.error != null;
      Object.assign(taskRead, { doc, sig, error: null });
    }
  } catch (e) {
    // The last document stays: a failed read is not an empty one.
    if (taskRead.key === key && epoch === navEpoch) {
      changed = taskRead.error !== e.message;
      taskRead.error = e.message;
    }
  } finally {
    taskRead.busy = false;
  }
  // An older link to the task's AI 下読み: its dialog opens once the document is in.
  if (pendingOwnRead === `${nav.board}/${task.id}` && taskRead.key === key && taskRead.doc) {
    pendingOwnRead = null;
    openOwnReadDialog(document.querySelector('#wk-term-head [data-own-read]'));
  }
  if (taskRead.again) {
    taskRead.again = false;
    taskReadRefresh(true);
  }
  if (changed) taskReadRedraw();
}

/* Draw what shows the task's read-through: the dialog while it is open, and the heading when its button says another thing. */
function taskReadRedraw() {
  const task = taskReadTaskOf();
  if (!task) return;
  const doc = taskRead.key === taskReadKey(task) ? taskRead.doc : null;
  if (taskReadHeadMode(task, doc, othersNowEst(), prStateOf(task)) !== taskRead.headMode) renderWorkTerm();
  if (taskReadActive(task)) cardDialogSync();
}

/* Ask for a read-through of the task's PR, or with `mode` 'split' for a split proposal. One request per PR at a time; the answer is
   read back either way, since a refusal (the hub stopped, a sync running) is also something the page now shows. */
async function taskReadAsk(mode) {
  const task = taskReadTask();
  const id = taskRead.doc?.pr;
  if (!task || !id || others.asking.has(id)) return;
  others.asking.add(id);
  taskReadRedraw();
  try {
    const res = await boardApi(BASE, `/api/tasks/${encodeURIComponent(task.id)}/ai`, { method: 'POST', body: JSON.stringify(mode ? { mode } : {}) });
    // The request stands; this is what the queue could not do after it.
    if (res?.warning) note('adj review request', true, res.warning);
  } catch (e) {
    note('adj review request', true, TASK_READ_WHY[e.why] || e.message);
  } finally {
    others.asking.delete(id);
    taskReadRedraw();
  }
  // Both are read again: the record is the task's, and the list may hold it too.
  await Promise.all([taskReadRefresh(true), refreshOthers(true)]);
}
/* What the refusals only the task's endpoint gives say, over the server's own wording. */
const TASK_READ_WHY = {
  syncing: 'ほかで同期しています。終わってからもう一度押してください',
  'pr-changed': 'タスクの PR が変わりました。もう一度押してください',
  'no-pr': 'このタスクには、まだ PR がありません',
};

/* ── the tabs ── */

/* Show `html` as the tabs of the row over the middle (the row itself is always shown, it leads with the sidebar's toggle), and either
   the read-through (`reading`) or the terminal of what is selected. Nothing has tabs now (a review request's read-through is one view,
   pr-panel.js), so `html` is blank. Returns whether the terminal pane has just been revealed, so that the caller can have it sized. */
function showMidTabs(html, reading) {
  const pane = wk('wk-term-pane');
  const read = wk('task-read');
  const tabs = wk('wk-mid-tabs');
  setWorkTermPart('tabs', tabs, html);
  tabs.hidden = !html;
  // The other view is not kept at a place to come back to by the page: it is the address's.
  const revealed = pane.hidden && !reading;
  pane.hidden = reading;
  read.hidden = !reading;
  return { revealed };
}

/* What is selected is not a review request: the terminal is the middle. A task with a PR has its document read when
   the selection comes to it (and on every poll after), which the heading's button reads. Returns what the caller needs to finish its draw. */
function taskReadFollow(task) {
  taskRead.shownFor = null;
  if (task?.pr && taskRead.key !== taskReadKey(task)) taskReadRefresh(true);
  return showMidTabs('', false);
}

/* ── the middle ── */

/* Say `html` in the middle in place of the read-through, and forget what the middle was drawn from, so that the next real draw
   replaces it whole (and starts at the reading place of its own). */
function taskReadSay(html) {
  const content = wk('task-read-content');
  taskRead.shownFor = null;
  taskRead.drawn.order = '';
  taskRead.groups = [];
  taskRead.contentSig = '';
  wk('task-read-band').innerHTML = '';
  wk('task-read-index').hidden = true;
  if (content.innerHTML !== html) content.innerHTML = html;
}

/* After a redraw: the start button had the keyboard, so it keeps it; once the hub is up the button is gone and the keyboard goes to
   the button that asks for a read. */
function keepHubStartFocus(held) {
  if (!held) return;
  const root = wk('task-read');
  const next = root.querySelector('[data-task-read-hub-start]') || root.querySelector('[data-pr-ai]:not([disabled]):not([aria-disabled="true"])');
  if (next) next.focus({ preventScroll: true });
}

/* Draw `groups` and the `band` over them in the middle, for what `key` names (a task's, or a review request's tab). What the middle was
   drawn from is forgotten when the key is another: cards are reused by their place, and another PR's diff card has the same place
   and, for a file both changed, the same markup, so it would be kept with the other PR's lines in it. `index` is whether the index is
   shown: with one group it is nothing to find one's way in. True when the content was drawn. */
function drawReadMiddle(key, groups, band, index = groups.length >= 2) {
  const body = wk('task-read-body');
  const content = wk('task-read-content');
  const hook = PR_HOOKS.diff;
  if (taskRead.drawnKey !== key) {
    taskRead.drawnKey = key;
    taskRead.contentSig = '';
    taskRead.drawn.order = '';
    taskRead.drawn.html = new Map();
  }
  const html = cardDialogTaskHtml(groups);
  const sig = `${band}\u0000${index}\u0000${html.index}\u0000${html.content}`;
  const drawn = sig !== taskRead.contentSig;
  if (drawn) {
    taskRead.contentSig = sig;
    taskRead.groups = groups;
    // Where the person is reading, and what they have open, survive the redraw.
    const restore = keepReadingPlace(body, content, taskRead.shownFor === key);
    wk('task-read-band').innerHTML = band;
    wk('task-read-index').innerHTML = html.index;
    wk('task-read-index').hidden = !index;
    prDialogPatch(content, cardDialogFill(html.content, groups, () => null), groups, taskRead.drawn);
    // The files that were drawn are drawn again from the cache, before the reading place is put back.
    hook?.onDrawnAt?.('read');
    restore(taskRead.shownFor === key);
  }
  // Something the middle was not drawn for, or the tab coming back: the reading place it had, else the top.
  if (taskRead.shownFor !== key) {
    taskRead.shownFor = key;
    body.scrollTop = taskRead.scroll.get(key) || 0;
  }
  hook?.applyTarget?.();
  taskReadSpy();
  return drawn;
}

/* ── the dialog ── */

/* Open the read-through of the task on show as a dialog (the heading's button, `opener`, or an older link). With `ask`, a read is
   asked for too, but only when one can be asked for now; otherwise the dialog only opens, and its band says why. */
function openOwnReadDialog(opener, { ask = false } = {}) {
  const dlg = cardDialogEl();
  const task = taskReadTaskOf();
  if (!dlg || dlg.open || !task?.pr) return;
  Object.assign(cardDialog, {
    mode: 'own-read', subject: task.id, pane: nav.pane, section: null, gate: null,
    opener, sig: '', held: false, quiet: false, gone: false, answering: false, gateGone: false, pressing: false,
    target: null, targetAt: 0, pinned: null, dock: null, dockSig: '', groups: [], gateIds: {}, active: null,
    answered: new Map(), answeredHere: new Set(), noMore: false, token: cardDialog.token + 1,
  });
  prDialogDrawn.order = '';
  cardDialogShell('own-read', ownReadTitleText(task), ownReadSubText(null, task, ownReadRepoHint(task)));
  ownReadDialogSync(true, true);
  dlg.showModal();
  dlg.querySelector('.card-dialog-body').scrollTop = 0;
  dlg.querySelector('.card-dialog-body').focus({ preventScroll: true });
  cardDialogSpy();
  const key = taskReadKey(task);
  const doc = taskRead.key === key ? taskRead.doc : null;
  // A PR too large to read is offered the split proposal in the dialog: the heading does not choose for the person.
  if (ask && doc?.pr && !othersOutOfReview(doc.record) && othersHubView(doc.hub, doc.record ? [doc.record] : []).canRead && !others.asking.has(doc.pr)) taskReadAsk();
}

/* What names the repository before there is a record: the PR's ref in the task's document, and the repository as the work document spells it. */
function ownReadRepoHint(task, doc = taskRead.key === taskReadKey(task) ? taskRead.doc : null) {
  return { ref: doc?.pr || '', nwo: workSelected()?.repo?.nwo || '' };
}

/* The text of the dialog's body when there is nothing to lay out in groups: one note, in a group of its own. */
const ownReadNoteGroup = html => ({ key: 'note', label: '下読みの状態', badge: '', cards: [{ key: 'note', label: '下読みの状態', badge: '', wide: true, html, expand: null }] });

/* `cardDialogSync` for a task's own AI 下読み: the head (with 「worker に渡す」), the band and the groups of its read-through in one scroll, with the
   person's comments on the lines of the diff. It follows the page while open (every poll of the task's document draws it), and closes quietly when the selection
   leaves the task. `force` redraws although nothing changed; `initial` is the first fill, before the dialog is open. */
function ownReadDialogSync(force = false, initial = false) {
  const dlg = cardDialogEl();
  if (!dlg.open && !initial) return;
  const task = taskReadTaskOf();
  if (!task || task.id !== cardDialog.subject) {
    cardDialog.quiet = true;
    if (dlg.open) dlg.close();
    return;
  }
  cardDialog.gone = false;
  const key = taskReadKey(task);
  const doc = taskRead.key === key ? taskRead.doc : null;
  const rec = doc?.record || null;
  const nowSecs = othersNowEst();
  const hubView = othersHubView(doc?.hub, rec ? [rec] : []);
  const asking = !!doc?.pr && others.asking.has(doc.pr);
  if (rec) prEnsureRead(rec);
  const read = rec ? prReadOf(rec) : null;
  // The PR's drafts are the file's, asked for at each opening (unless something is unsaved), and the dialog is drawn when they come.
  const opening = `${key}#${cardDialog.token}`;
  if (rec && taskRead.draftsFor !== opening) {
    taskRead.draftsFor = opening;
    const e = prDraftsEntry(rec.id);
    if (!e.dirty && !e.saving && !e.handing) { e.loaded = false; e.error = null; }
  }
  if (rec) prEnsureDrafts(rec.id);

  const title = dlg.querySelector('#card-dialog-title');
  if (title.textContent !== ownReadTitleText(task)) title.textContent = ownReadTitleText(task);
  const sub = dlg.querySelector('#card-dialog-sub');
  const subText = ownReadSubText(rec, task, ownReadRepoHint(task, doc));
  if (sub.textContent !== subText) sub.textContent = subText;
  // What 「worker に渡す」 counts: the comments a hand-over would send now, once the drafts are loaded.
  const sendN = rec && prHasRead(rec, read) && prDraftsEntry(rec.id).loaded ? prHandable(prDoc(rec.id, read || null)).drafts : null;
  cardDialogHead({ line: ownReadHeadLine(doc, nowSecs), acts: ownReadActsHtml(rec, read, hubView, asking, !!rec && others.closing.has(rec.id), sendN) });

  // A poll that failed keeps the last document: say that what is shown may be behind.
  const failed = doc && taskRead.error
    ? `<div class="wk-notice" role="alert">${esc(`最新の状態を読み込めませんでした（前に読んだ状態を表示しています）: ${taskRead.error}`)}</div>` : '';
  let groups;
  if (!doc || !doc.pr || !rec) {
    groups = [ownReadNoteGroup(failed + (!doc ? (taskRead.error ? `<div class="wk-notice" role="alert">${esc(`読み込めませんでした: ${taskRead.error}`)}</div>` : '<div class="tp-muted" role="status">読み込み中…</div>')
      : !doc.pr ? `<div class="m3-filled-card"><div class="tp-muted">${esc(TASK_READ_UNREADABLE_WHY)}</div></div>`
        // The same block the review list's middle has before a first read, under what the stopped hub says.
        : taskReadHubHtml(null, hubView, others.starting) + prNoReadHtml(null, { kind: 'none', text: '', action: '下読みする' }, hubView, asking)))];
  } else {
    const band = failed + taskReadHubHtml(rec, hubView, others.starting) + taskReadBandHtml(rec, read, hubView, asking);
    if (!prHasRead(rec, read) && !othersOutOfReview(rec)) groups = [ownReadNoteGroup(band + prNoReadHtml(rec, othersAiView(rec, [], nowSecs), hubView, asking))];
    else {
      const hook = PR_HOOKS.diff;
      // A split proposal did not read the diff: none is fetched for it.
      const diffOn = !!hook && !!read && read.mode !== 'split';
      groups = taskReadGroups(rec, read, hubView, nowSecs, {
        terminal: false, diff: !!hook, asking, readError: !!prReadEntry(rec.id)?.error,
        diffFileHtml: diffOn ? path => hook.dialogFileHtml(rec, read, path) : null,
        diffPaths: diffOn ? hook.dialogPaths(rec, read) : null,
        diffHeadHtml: diffOn ? hook.dialogHeadHtml(rec, read) : null,
        diffLabel: diffOn ? path => hook.dialogLabel(rec, path) : null,
      });
      if (band) groups.unshift(ownReadNoteGroup(band));
    }
  }
  prDialogPaint(groups, '', { force, initial, loading: read === undefined });
}

/* The dialog's own clicks, on top of the review request's buttons: the hub's start button and 「下読みを閉じる」. */
function ownReadDialogClick(e) {
  const start = e.target.closest('[data-task-read-hub-start]');
  if (start) {
    if (start.getAttribute('aria-disabled') !== 'true') othersStartHub();
    return;
  }
  // A button that asks for a read works before the PR has a record too, which `prAct` needs for everything else.
  const b = e.target.closest('[data-pr-ai], [data-pr-split], [data-pr-reread]');
  if (b) {
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') taskReadAsk(b.matches('[data-pr-split]') ? 'split' : undefined);
    return;
  }
  if (e.target.closest('[data-pr-close-reader]')) return prCloseReader();
  prAct(e, { place: 'dialog' });
}

/* The dialog was closed: what was written is saved now. */
function ownReadDialogClosed() {
  const id = taskRead.doc?.pr;
  if (id) prScheduleSave(id, true);
}

/* The page follows its state: the open dialog is drawn again, or closed when the selection is no longer its task. */
function ownReadDialogFollow() {
  if (cardDialog.mode === 'own-read' && cardDialogEl().open) cardDialogSync();
}
/* The work view is left: the dialog goes with it. */
function closeOwnReadDialog() {
  if (cardDialog.mode !== 'own-read' || !cardDialogEl().open) return;
  cardDialog.quiet = true;
  cardDialogEl().close();
}

/* The index marks the group being read. */
function taskReadSpy() {
  if (!taskRead.groups.length) return;
  const spied = markIndexCurrent({
    index: wk('task-read-index'), body: wk('task-read-body'), content: wk('task-read-content'), pinned: taskRead.pinned, was: taskRead.active,
  });
  if (spied) taskRead.active = spied.key;
}

/* Bring a card (or a group) to the top, as the index and a link to a line of the diff do. */
function taskReadGo(cardKey, groupKey, flash) {
  const hit = scrollToCard(wk('task-read-content'), wk('task-read-body'), cardKey, groupKey, flash);
  if (!hit) return false;
  // The index marks where the person went until they scroll.
  taskRead.pinned = groupKey || taskRead.groups.find(g => g.cards.some(c => c.key === cardKey))?.key || null;
  taskRead.pinnedAt = hit.at;
  taskReadSpy();
  return true;
}

wk('task-read-index').addEventListener('click', e => {
  const b = e.target.closest('[data-cd-go-group]');
  if (!b) return;
  taskReadGo(b.dataset.cdGoCard || null, b.dataset.cdGoGroup, false);
});

// A scroll by hand ends the wait for a place and lets go of the group a jump pinned; the page's own scrolling does not come through these.
const taskReadUserMoved = () => { taskRead.pinned = null; PR_HOOKS.diff?.onMoved?.(); };
for (const type of ['wheel', 'touchstart', 'pointerdown']) wk('task-read-body').addEventListener(type, taskReadUserMoved, { passive: true });
wk('task-read-body').addEventListener('keydown', e => {
  if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(e.key)) taskReadUserMoved();
});
wk('task-read-body').addEventListener('scroll', e => {
  const body = e.currentTarget;
  if (taskRead.shownFor) taskRead.scroll.set(taskRead.shownFor, body.scrollTop);
  // Away from where a jump left it: the person has scrolled, however they did it.
  if (taskRead.pinned && Math.abs(body.scrollTop - taskRead.pinnedAt) > 8) taskRead.pinned = null;
  if (taskRead.spying) return;
  taskRead.spying = true;
  requestAnimationFrame(() => { taskRead.spying = false; taskReadSpy(); });
});

/* The buttons of the middle are the review request's: a button that asks for a read goes to what is on show, the rest to `prAct`. */
wk('task-read').addEventListener('click', e => {
  const start = e.target.closest('[data-task-read-hub-start]');
  if (start) {
    if (start.getAttribute('aria-disabled') !== 'true') othersStartHub();
    return;
  }
  const b = e.target.closest('[data-pr-ai], [data-pr-split], [data-pr-reread]');
  if (b) {
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') prShown()?.ask(b.matches('[data-pr-split]') ? 'split' : undefined);
    return;
  }
  prAct(e, { place: 'read' });
});
// What is typed in a comment's box on a line of a review request's middle; a task's are in its dialog.
wk('task-read').addEventListener('input', e => { if (prShown()?.kind === 'pr') PR_HOOKS.diff?.input?.(e); });

// A link to a line of the diff lands in the middle, which is already as large as it can be.
PRD_VIEWS.read = {
  ready: () => !!prShown() && !wk('task-read').hidden && nav.pane === 'detail',
  go: (...a) => taskReadGo(...a),
  cardEl: key => [...wk('task-read-content').querySelectorAll('[data-cd-card]')].find(el => el.dataset.cdCard === key) || null,
  place: 'read',
};

/* ── the panel: the reader ── */

/* The reader's terminal in the panel, and what stands over it while there is none: `missingHtml` while the PR has no record, else
   what a worker that is not there, has ended or could not be reached is told (`prMiddleHtml`, with the queue in `records`). `mount` is
   false while the terminal's pane is hidden: it has no size to fit to. */
function drawReaderTerm(rec, records, hubView, missingHtml, mount = true) {
  syncReaderTerm(panelTerm, rec, prReaderSubject(rec), mount);
  setPanelPart('bar', tp('tp-term-bar'), '');
  const ph = tp('tp-term-ph');
  const live = !!panelTerm.term && panelTerm.ended == null;
  ph.hidden = live;
  const text = live ? '' : !rec ? missingHtml
    : prMiddleHtml(rec, records, hubView, othersNowEst(), { asking: others.asking.has(rec.id), read: prReadOf(rec), readError: !!prReadEntry(rec.id)?.error, ...prTermEnd(panelTerm) });
  setPanelPart('ph', ph, text);
}
