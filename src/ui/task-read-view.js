/* ── 「AI 下読み」 of a task: the tab over the middle, the read-through in it, and the reader in the panel ──────────────────────────
   A task whose PR has a read-through (docs/architecture.md, "A task's PR") has two views in the work view, chosen by the tabs over
   the middle: 「タスク」, which is the worker's terminal in the middle and the task's panel, and 「AI 下読み」, which is the
   read-through in the middle, laid out as 「拡大」 lays out a review request's, and the reader (its terminal, and 「worker への指示」)
   in the panel. The address carries the tab (`pane=ai`, `pane=ai-drafts`), so a reload and a link keep it. What the markup says is
   task-read.js's (pure) and, below it, pr-read.js's; what is drawn is the dialog's machinery (`cardDialogTaskHtml`,
   `prDialogPatch`), and the buttons are the review request's (`prAct`, with `place: 'read'`). This file keeps what the page holds
   of it: the task's document, its poll, and the reading place. Loaded after pr-diff-view.js. */

const taskRead = {
  key: null,          // `<board>/<task>/<pr>`: what `doc` is for; an answer is used only while this is still the task on show
  doc: null,          // `GET /api/tasks/<id>/ai`: { pr, record, hub }, null until it has come
  sig: '',            // `doc` as JSON, so that an answer that says what was said draws nothing
  error: null,        // why the last read failed
  busy: false,        // a read is out
  again: false,       // a forced read asked for while one was out
  taskPane: new Map(),  // `<board>/<task id>` -> the pane of the task's panel before this tab was chosen, which 「タスク」 goes back to
  drawn: { order: '', html: new Map() },  // what the middle was last drawn from (`prDialogPatch`)
  contentSig: '',     // the middle's markup as drawn
  groups: [],         // the groups drawn
  shownFor: null,     // the key the middle is drawn for while it is on screen, null while the other tab is
  draftsFor: null,    // the key the PR's drafts were last asked for while the tab was on: asked again when the tab is chosen again
  scroll: new Map(),  // key -> the middle's scroll top
  active: null,       // the group the index marks
  pinned: null,       // the group a jump left, marked until the person scrolls
  pinnedAt: 0,
  spying: false,
};

/* What task-read-drafts.js fills in to put the drafts in the reader's panel: `html(rec, read)` is what 「worker への指示」 shows,
   `act(e)` answers a click in the panel (true when it was one of its own) and `input(e)` an input. While `drafts` is set the
   middle's draft buttons are drawn (`data-drafts` on #task-read). */
const OWN_HOOKS = { drafts: null };

/* The tab is on: the work view is on a task that has a PR, and the address names one of the read panes. */
const taskReadActive = task => !!task && view === 'work' && READ_PANES.includes(nav.pane) && !!task.pr && task.id === nav.task;
const taskReadTaskOf = () => view === 'work' && isTaskRef(nav.task) ? taskById(nav.task) || null : null;
const taskReadTask = () => { const t = taskReadTaskOf(); return taskReadActive(t) ? t : null; };
const taskReadKey = task => `${nav.board}/${task.id}/${task.pr}`;

/* What the read-through's buttons and terminal are on, for `prShown` (pr-panel.js): null until the task's document has come, or
   when its PR is not one that can be read. `rec` is null while the PR has no record. */
function taskReadShown() {
  const task = taskReadTask();
  if (!task || taskRead.key !== taskReadKey(task) || !taskRead.doc?.pr) return null;
  const { pr: id, record: rec, hub } = taskRead.doc;
  return { id, rec, kind: 'task', hubView: othersHubView(hub, rec ? [rec] : []), asking: others.asking.has(id), ask: mode => taskReadAsk(mode), slot: panelTerm };
}

/* ── the document ── */

/* Read the task's document when it has a PR: the tab shows its state beside its label, so this is asked for in either tab, on every
   poll while the task is selected (a read in flight is not asked again). A task that is not the one the document is for starts
   it over. */
async function taskReadRefresh(force = false) {
  const task = taskReadTaskOf();
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
  if (taskRead.again) {
    taskRead.again = false;
    taskReadRefresh(true);
  }
  if (changed) taskReadRedraw();
}

/* Draw what shows the task's read-through: the tabs always, and the panel (which draws the middle) while the tab is on. */
function taskReadRedraw() {
  const task = taskReadTaskOf();
  if (!task) return;
  if (taskReadActive(task) && selectedTaskId === task.id) renderTaskPanel(); else renderTaskReadTabs(task);
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

/* The tabs of the row over the middle (the row itself is always shown, it leads with the sidebar's toggle), and which of the
   two views is shown. Returns what the caller needs to finish its own draw. */
function renderTaskReadTabs(task) {
  const pane = wk('wk-term-pane');
  const read = wk('task-read');
  if (!task) {
    // A selection that is not a task has no tabs (the row stays, for the toggle), and is the middle as it always was.
    wk('wk-mid-tabs').hidden = true;
    setWorkTermPart('tabs', wk('wk-mid-tabs'), '');
    read.hidden = true;
    taskRead.shownFor = null;
    const revealed = pane.hidden;
    pane.hidden = false;
    return { reading: false, revealed };
  }
  if (task.pr && taskRead.key !== taskReadKey(task)) taskReadRefresh(true);
  const reading = taskReadActive(task);
  const doc = taskRead.key === taskReadKey(task) ? taskRead.doc : null;
  const focused = document.activeElement?.closest?.('#wk-mid-tabs [data-mid-tab]')?.dataset.midTab;
  const before = workTermPart.tabs;
  setWorkTermPart('tabs', wk('wk-mid-tabs'), taskReadTabsHtml(taskReadTabState(task, doc, othersNowEst()), reading ? 'ai' : 'task'));
  if (focused && workTermPart.tabs !== before) wk(`wk-mid-tab-${focused}`)?.focus({ preventScroll: true });
  wk('wk-mid-tabs').hidden = false;
  // The other view is not kept at a place to come back to by the page: it is the address's.
  const revealed = pane.hidden && !reading;
  pane.hidden = reading;
  read.hidden = !reading;
  if (!reading) taskRead.shownFor = taskRead.draftsFor = null;
  return { reading, revealed };
}

wk('wk-mid-tabs').addEventListener('click', e => {
  const b = e.target.closest('[data-mid-tab]');
  const task = taskReadTaskOf();
  if (!b || b.disabled || !task) return;
  const on = taskReadActive(task);
  if (b.dataset.midTab === 'ai') {
    if (on) return;
    // 「タスク」 goes back to the tab of the task's panel that was on.
    taskRead.taskPane.set(`${nav.board}/${task.id}`, nav.pane);
    go({ pane: 'ai' }, { replace: true });
  } else if (on) {
    go({ pane: taskRead.taskPane.get(`${nav.board}/${task.id}`) || 'detail' }, { replace: true });
  }
});

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

/* The no-record card, with what the stopped hub says (and its start button) over it. */
function taskReadNoRecordWithHubHtml(pr, hubView) {
  const hub = taskReadHubHtml(null, hubView, others.starting);
  return (hub ? `<div class="m3-filled-card pr-sec">${hub}</div>` : '') + taskReadNoRecordHtml(pr, hubView, others.asking.has(pr));
}

/* The middle of the 「AI 下読み」 tab. Called from `renderWorkTerm`, which has made the pane visible. */
function renderTaskReadMiddle(task) {
  const key = taskReadKey(task);
  const doc = taskRead.key === key ? taskRead.doc : null;
  const body = wk('task-read-body');
  wk('task-read').dataset.drafts = OWN_HOOKS.drafts ? 'on' : 'off';
  const rec = doc?.record || null;
  const startHeld = !!document.activeElement?.matches?.('[data-task-read-hub-start]');
  if (!doc || !doc.pr || !rec) {
    const hubView = othersHubView(doc?.hub, []);
    // A poll that failed keeps the last document: say that what is shown may be behind.
    const behind = doc && taskRead.error ? `<div class="wk-notice" role="alert">${esc(`最新の状態を読み込めませんでした（前に読んだ状態を表示しています）: ${taskRead.error}`)}</div>` : '';
    taskReadSay(behind + (!doc ? (taskRead.error ? `<div class="wk-notice" role="alert">${esc(`読み込めませんでした: ${taskRead.error}`)}</div>` : '<div class="tp-muted" role="status">読み込み中…</div>')
      : !doc.pr ? `<div class="m3-filled-card"><div class="tp-muted">${esc(TASK_READ_UNREADABLE_WHY)}</div></div>`
        : taskReadNoRecordWithHubHtml(doc.pr, hubView)));
    keepHubStartFocus(startHeld);
    return;
  }

  prEnsureRead(rec);
  const read = prReadOf(rec);
  const hubView = othersHubView(doc.hub, [rec]);
  const asking = others.asking.has(rec.id);
  const hook = PR_HOOKS.diff;
  const groups = taskReadGroups(rec, read, hubView, othersNowEst(), {
    diff: !!hook, asking, readError: !!prReadEntry(rec.id)?.error,
    diffFileHtml: hook ? path => hook.dialogFileHtml(rec, read, path) : null,
    diffPaths: hook && read ? hook.dialogPaths(rec, read) : null,
    diffHeadHtml: hook && read ? hook.dialogHeadHtml(rec, read) : null,
    diffLabel: hook ? path => hook.dialogLabel(rec, path) : null,
  });
  const html = cardDialogTaskHtml(groups);
  // A poll that failed keeps the last document, so say that what is shown may be behind.
  const failed = taskRead.error
    ? `<div class="wk-notice" role="alert">${esc(`最新の状態を読み込めませんでした（前に読んだ状態を表示しています）: ${taskRead.error}`)}</div>`
    : '';
  const band = failed + taskReadHubHtml(rec, hubView, others.starting) + taskReadBandHtml(rec, read, hubView, asking);
  const sig = `${band}\u0000${html.index}\u0000${html.content}`;
  const content = wk('task-read-content');
  if (sig !== taskRead.contentSig) {
    taskRead.contentSig = sig;
    taskRead.groups = groups;
    // Where the person is reading, and what they have open, survive the redraw.
    const restore = keepReadingPlace(body, content, taskRead.shownFor === key);
    wk('task-read-band').innerHTML = band;
    wk('task-read-index').innerHTML = html.index;
    // One group is nothing to find one's way in.
    wk('task-read-index').hidden = groups.length < 2;
    prDialogPatch(content, cardDialogFill(html.content, groups, () => null), groups, taskRead.drawn);
    // The files that were drawn are drawn again from the cache, before the reading place is put back.
    hook?.onDrawnAt?.('read');
    restore(taskRead.shownFor === key);
    keepHubStartFocus(startHeld);
  }
  // A task the middle was not drawn for, or the tab coming back: the reading place it had, else the top.
  if (taskRead.shownFor !== key) {
    taskRead.shownFor = key;
    body.scrollTop = taskRead.scroll.get(key) || 0;
  }
  hook?.applyTarget?.();
  taskReadSpy();
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
  const card = b.dataset.cdGoCard || null;
  // The category of a file is turned on before the jump, or the jump lands on a hidden card.
  if (card?.startsWith('diff:')) prdShowCategoryOf(card.slice('diff:'.length));
  taskReadGo(card, b.dataset.cdGoGroup, false);
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

/* The buttons of the middle are the review request's: a button that asks for a read goes to the task, the rest to `prAct`. */
wk('task-read').addEventListener('click', e => {
  const start = e.target.closest('[data-task-read-hub-start]');
  if (start) {
    if (start.getAttribute('aria-disabled') !== 'true') othersStartHub();
    return;
  }
  const b = e.target.closest('[data-pr-ai], [data-pr-split], [data-pr-reread]');
  if (b) {
    if (!b.disabled && b.getAttribute('aria-disabled') !== 'true') taskReadAsk(b.matches('[data-pr-split]') ? 'split' : undefined);
    return;
  }
  if (OWN_HOOKS.drafts?.act?.(e)) return;
  prAct(e, { place: 'read' });
});
wk('task-read').addEventListener('input', e => { OWN_HOOKS.drafts?.input?.(e); });

// A link to a line of the diff lands in the middle, which is already as large as it can be.
PRD_VIEWS.read = {
  ready: () => !!taskReadTask() && !wk('task-read').hidden,
  go: (...a) => taskReadGo(...a),
  cardEl: key => [...wk('task-read-content').querySelectorAll('[data-cd-card]')].find(el => el.dataset.cdCard === key) || null,
  place: 'read',
};

/* ── the panel: the reader ── */

/* The panel of the 「AI 下読み」 tab: the reader's terminal and 「worker への指示」. The terminal stays mounted behind the second tab,
   hidden, so its socket is kept. */
function renderTaskReaderPanel(task) {
  const key = taskReadKey(task);
  const doc = taskRead.key === key ? taskRead.doc : null;
  const rec = doc?.record || null;
  panelShown = null;
  panelScrolledFor = null;
  renderWorkAway(null);
  renderHandForm(null);
  tp('tp-form').hidden = true;
  tp('tp-tabs').hidden = false;
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), '');

  const read = rec ? prReadOf(rec) : null;
  const hasRead = rec ? prHasRead(rec, read) : false;
  // 「worker への指示」 needs a read-through: a link to it before there is one is the terminal.
  const pane = nav.pane === 'ai-drafts' && hasRead ? 'ai-drafts' : 'ai';
  setPanelPart('head', tp('tp-head'), rec ? taskReaderHeadHtml(rec, others.closing.has(rec.id))
    : `<div class="tp-head-main"><h2 class="tp-title">下読みの worker</h2></div>${panelBtnsHtml('')}`);
  // The PR's drafts are the file's, asked for when the tab is chosen (and again after it was left, unless something is unsaved), and the
  // panel is drawn when they come.
  if (rec && taskRead.draftsFor !== key) {
    taskRead.draftsFor = key;
    const e = prDraftsEntry(rec.id);
    if (!e.dirty && !e.saving && !e.handing) { e.loaded = false; e.error = null; }
  }
  if (rec) prEnsureDrafts(rec.id);
  // What is left to hand over once the drafts are in; until then what the read-through brought.
  const held = rec && hasRead && read !== undefined && prDraftsEntry(rec.id).loaded ? prHandable(prDoc(rec.id, read)).drafts : rec?.ai?.counts?.drafts;
  setPanelPart('tabs', tp('tp-tabs'), taskReaderTabsHtml(pane, Number.isFinite(held) ? held : 0, hasRead));

  // Shown before the terminal is mounted: a hidden host has no size to fit to.
  const reveal = pane === 'ai' && tp('tp-term').hidden;
  tp('tp-detail').hidden = pane === 'ai';
  tp('tp-term').hidden = pane !== 'ai';
  syncReaderTerm(panelTerm, rec, prReaderSubject(rec), pane === 'ai');
  setPanelPart('bar', tp('tp-term-bar'), '');
  const ph = tp('tp-term-ph');
  const live = !!panelTerm.term && panelTerm.ended == null;
  ph.hidden = live;
  const records = rec ? [rec] : [];
  const text = live ? ''
    : !doc ? `<div class="tp-muted">${esc(taskRead.error ? `読み込めませんでした: ${taskRead.error}` : '読み込み中…')}</div>`
      : !rec ? '<div class="tp-ph-head">AI 下読みの worker はまだいません</div><div class="tp-muted">AI 下読みすると、ここにその worker のターミナルが開きます。</div>'
        // No records for the queue's place: the document has this record only, which would always be the first.
        : prMiddleHtml(rec, [], othersHubView(doc.hub, records), othersNowEst(), {
          readAt: 'middle', asking: others.asking.has(rec.id), read, readError: !!prReadEntry(rec.id)?.error, ...prTermEnd(panelTerm),
        });
  setPanelPart('ph', ph, text);
  const restHtml = pane === 'ai-drafts' && rec && OWN_HOOKS.drafts ? OWN_HOOKS.drafts.html(rec, read) : '';
  const restKey = `${key}/${pane}`;
  // What is typed in a box is kept: the redraw waits until the box is left (prHeldFlush).
  if (restHtml && prBoxFocused(tp('task-panel')) && panelSig.paneKey === restKey && !prView.force) prView.held = true;
  else {
    prView.held = false;
    const keep = prFocusKeep(tp('tp-rest'));
    const scroll = tp('tp-detail').scrollTop;
    setPanelPart('rest', tp('tp-rest'), restHtml);
    prRestoreFocus(tp('tp-rest'), keep, false);
    tp('tp-detail').scrollTop = panelSig.paneKey === restKey ? scroll : 0;
    panelSig.paneKey = restKey;
  }
  // It was sized while hidden, which it skips; asked again now that it has a size.
  if (reveal && panelTerm.term) panelTerm.term.fit();
}

/* The panel's own clicks in the 「AI 下読み」 tab. */
function taskReaderPanelClick(e) {
  let b;
  if (e.target.closest('[data-tp-close]')) return closeTaskPanel();
  if ((b = e.target.closest('[data-read-pane]'))) {
    if (!b.disabled) go({ pane: b.dataset.readPane }, { replace: true });
    return;
  }
  if (e.target.closest('[data-pr-close-reader]')) return prCloseReader();
  if (e.target.closest('[data-pr-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
    return;
  }
  if (OWN_HOOKS.drafts?.act?.(e)) return;
  prAct(e, { place: 'read' });
}
tp('task-panel').addEventListener('input', e => {
  if (taskReadActive(taskById(selectedTaskId))) OWN_HOOKS.drafts?.input?.(e);
});
