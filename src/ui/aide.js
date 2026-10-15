/* ── 副官: the middle of 「いまの仕事」 when nothing is selected ─────────────────────────────────────
   The aide's terminal on the left and the display area on the right (the pure half is aide-display.js), the pinned 「副官」 row of
   the list, the bar and the count that bring the person back from a session it sent them to, and the buttons that ask it
   something. The address is `/?view=work` with no task: no new parameter, so Back from a session lands here.

   Showing the pane never starts the aide: Claude Code asks once whether to trust its directory, and that question needs a
   person looking. Only the 「副官」 row, 「副官を起動」 and 「副官に聞く」 call `POST /api/aide/open`. `GET /api/aide` runs a process
   check, so it is read only while the pane is on screen (core.js `refresh`). */

const aide = {
  view: null,        // GET /api/aide, as last read; kept when a read fails
  error: null,
  busy: false,
  again: false,      // a forced round asked for while one was out
  reads: 0,          // reads of /api/aide sent so far
  readOk: 0,         // which of them was the last that worked; a queued request is typed only from one sent after it
  askAfter: 0,       // how many reads had been sent when the queued request was made
  display: aideEmptyDisplay(),
  opening: false,
  resetting: false,
  wantOpen: false,   // the person asked for it to be started; acted on when a read says it can be
  pendingAsk: null,  // a request typed once the aide can take it (aideFlushAsk)
  from: null,        // { board, ref, title }: the row the aide sent the person to, for the way back; page memory, not the address
  seen: null,        // the keys of the waits the pane last showed, for 「待ちが N 件増えました」; null before it was shown
};
/* The aide's terminal: a slot like the middle's (`workTerm`), connected through the aide's own socket and not a board's. */
const aideTerm = { host: () => wk('aide-term-host'), redraw: () => renderAide(), base: () => '',
  taskId: null, sessionId: null, term: null, ended: null, endReason: null, reconnect: false,
  // The terminal takes the keyboard only when the person asked for the aide, not when a poll mounts it.
  focus: false };
/* The markup each part of the pane was last given, so a poll that changes nothing leaves the focus and an open <details> alone. */
const aideDrawn = {};
function setAidePart(name, el, html) {
  if (!el || aideDrawn[name] === html) return;
  aideDrawn[name] = html;
  el.innerHTML = html;
}

/* What the display area shows: the state the aide's adjutant_show tool left, which the resident reads from its file. */
async function loadAideDisplay() {
  return boardApi('', '/api/aide/display');
}

/* ── what the work document says ── */

const aideCurrentWaits = () => (work.doc ? aideWaits(workEntries(work.doc, key => answeredGates.has(key))) : []);

const aideRowTitle = r => r.task?.title || (r.turn
  ? (r.entry.session ? sessionLabel(r.entry.session, false, r.data).text : r.entry.gateTitle || r.ref)
  : sessionLabel(r.s, false, r.data).text);

/* The rows of 新着 as the list draws them, and the first of them with what it is in. A hub is no row of it. */
function aideNews() {
  if (!work.doc) return null;
  const { rows, turns } = workListRows();
  const at = r => { const i = work.newOrder.indexOf(r.id); return i < 0 ? Infinity : i; };
  const fresh = [...rows, ...turns].filter(r => r.cls === 'new' && !r.isHub).sort((a, b) => at(a) - at(b));
  if (!fresh.length) return null;
  const r = fresh[0];
  const gate = (r.live || []).find(i => i.kind === 'gate');
  const pr = (r.live || []).find(i => i.kind === 'pr');
  const kindLabel = gate ? kindOf(gate.gate)[0] : '';
  const what = kindLabel || (pr && WORK_PR_NOW[pr.turn]) || WORK_GLYPH[r.st]?.[1] || '';
  return { count: fresh.length, first: { title: aideRowTitle(r), what, key: r.key, nwo: r.nwo, taskId: r.task?.id || null, kindLabel } };
}

/* ── reading and starting ── */

async function refreshAide(force = false) {
  if (!multiBoard) return;
  if (aide.busy) {
    // An answer to something just done must not be lost to the round already out.
    if (force) aide.again = true;
    return;
  }
  aide.busy = true;
  try {
    const n = ++aide.reads;
    aide.view = await boardApi('', '/api/aide');
    aide.readOk = n;
    aide.error = null;
    try {
      aide.display = aideDisplayOf(await loadAideDisplay());
    } catch {
      // The area keeps what it last showed. A display that cannot be read says nothing about the aide, so it is not the pane's error.
    }
  } catch (e) {
    // The last read stays on screen, with the reason it is old.
    aide.error = e.message;
  } finally {
    aide.busy = false;
  }
  const v = aide.view;
  if (v && aide.wantOpen) {
    // Never on `unknown`: a second aide must not start over one that cannot be told apart from a live one.
    if ((v.state === 'none' || v.state === 'stopped') && v.startable) openAide();
    else aide.wantOpen = false;
  }
  if (v && aide.pendingAsk && !v.startable && v.state !== 'running' && v.state !== 'starting') {
    aide.pendingAsk = null;
    note('副官に聞く', true, AIDE_NOT_STARTABLE);
  }
  renderAide();
  aideFlushAsk();
  if (aide.again) {
    aide.again = false;
    refreshAide(true);
  }
}

async function openAide() {
  if (aide.opening) return;
  aide.opening = true;
  aide.wantOpen = false;
  try {
    await boardApi('', '/api/aide/open', { method: 'POST', body: '{}' });
  } catch (e) {
    aide.pendingAsk = null;
    note('副官を起動できません', true, e.status === 409 ? AIDE_NOT_STARTABLE : e.message);
  } finally {
    aide.opening = false;
  }
  refreshAide(true);
}

function resetAide() {
  if (aide.resetting) return;
  aide.resetting = true;
  renderAide();
  boardApi('', '/api/aide/reset', { method: 'POST', body: '{}' }).then(r => {
    // The old socket is of the session that was just stopped; the new one is mounted when a read says it runs.
    disposeTermSlot(aideTerm);
    note('副官をリセットしました', false, r.started === false ? '止めましたが、新しい副官は起動できませんでした' : r.wasBusy ? '作業の途中でしたが止めました' : '新しいセッションで起動しています');
  }, e => {
    note('副官をリセットできません', true, e.status === 409 ? AIDE_NOT_STARTABLE : e.message);
  }).finally(() => {
    aide.resetting = false;
    refreshAide(true);
  });
}

/* ── the terminal ── */

/* Mounted only while the pane is shown and the aide runs: a hidden host has no size to fit to, and a terminal for a session that
   is not there yet would only be told 4404. One already mounted is kept while a row is open, and is the one the way back shows. */
function syncAideTerm(visible) {
  const v = aide.view;
  // A read that could not tell is no reason to drop a socket that works.
  const subject = !v || v.state === 'unknown' ? aideTerm.taskId : v.state === 'running' ? v.sessionId : null;
  if (aideTerm.taskId !== subject || (aideTerm.reconnect && subject)) disposeTermSlot(aideTerm);
  aideTerm.taskId = subject;
  if (!subject || aideTerm.term || !visible) return;
  aideTerm.sessionId = subject;
  aideTerm.ended = null;
  aideTerm.endReason = null;
  const handle = mountSessionTerminal(aideTerm.host(), {
    path: `/api/aide/terminal?session=${encodeURIComponent(subject)}`,
    base: '',
    focus: aideTerm.focus,
    onEnd: (code, reason) => {
      if (aideTerm.term !== handle) return;
      aideTerm.ended = code;
      aideTerm.endReason = reason;
      // 4410: it ended or another took its place; the read says which, and a new session is mounted by the next draw.
      if (code === 4410) refreshAide(true);
      renderAide();
    },
  });
  aideTerm.term = handle;
  aideTerm.focus = false;
}

/* What is under the terminal while there is none, or what it still has to say. */
function aidePlaceholderHtml() {
  const v = aide.view;
  const muted = text => `<div class="tp-muted">${esc(text)}</div>`;
  const start = '<button type="button" class="btn-m3-primary" data-aide-start>副官を起動</button>';
  const lines = [];
  if (aideTerm.term) {
    if (aideTerm.ended != null) {
      lines.push(muted(`副官のターミナルが閉じました: ${terminalEndText(aideTerm.ended, aideTerm.endReason)}`),
        '<button type="button" class="btn-m3-tonal" data-aide-reconnect>再接続</button>');
    }
  } else if (!v) lines.push(muted(aide.error ? `副官の状態を読めませんでした: ${aide.error}` : '読み込み中…'));
  else if (v.state === 'starting') lines.push(muted('副官を起動しています…'));
  else if (v.state === 'running') lines.push(muted('ターミナルに接続しています…'));
  else if (v.state === 'unknown') lines.push(muted(`副官の状態を読めません: ${v.error || '理由は不明です'}`));
  else if (!v.startable) lines.push(muted(AIDE_NOT_STARTABLE));
  else {
    lines.push(muted(v.state === 'stopped' ? '副官は止まっています' : '副官はまだ起動していません'), start,
      muted('最初の起動では、Claude Code がこのフォルダを信頼するかをここで尋ねます'));
  }
  if (v && aide.error) lines.push(muted(`副官の状態を読めませんでした: ${aide.error}`));
  if (aide.pendingAsk) {
    lines.push(muted(`副官の手が空いたら、次を送ります: 「${aide.pendingAsk}」`),
      '<button type="button" class="btn-m3-text" data-aide-ask-cancel>取り消す</button>');
  }
  return lines.join('');
}

/* 「対象」: the topic on display, and a chip for each repository, which puts `[owner/repo] ` at the start of what is typed. */
function aideScopeHtml() {
  const label = aide.display.topic?.label || 'いまの話題なし';
  const repos = workRepos().filter(r => !r.error).slice().sort((a, b) => a.nwo.localeCompare(b.nwo, undefined, { sensitivity: 'base' }));
  const chip = (scope, text, title) => `<button type="button" class="aide-chip" data-aide-scope="${esc(scope)}" title="${esc(title)}">${esc(text)}</button>`;
  return `<span class="aide-scope-label">${aideIcon('my_location')}対象</span>`
    + `<span class="aide-scope-cur" title="何も付けずに聞くと、いま表示領域に出ている話題の続きとして扱います">${esc(label)}</span>`
    + repos.map(r => chip(r.nwo, workRepoName(r.nwo), `[${r.nwo}] を入れて、このリポジトリの話として聞く`)).join('')
    + chip('#', '#…', 'タスク番号を入れる');
}

/* ── drawing ── */

function renderAide() {
  if (view !== 'work') return;
  const on = !nav.task;
  wk('aide').hidden = !on;
  wk('wk-term').classList.toggle('aide-on', on);
  if (on) {
    setAidePart('head', wk('aide-head'), aideHeadHtml(aide.view, { resetting: aide.resetting }));
    syncAideTerm(true);
    const ph = wk('aide-term-ph');
    setAidePart('ph', ph, aidePlaceholderHtml());
    ph.hidden = !aideDrawn.ph;
    setAidePart('scope', wk('aide-scope'), aideScopeHtml());
    setAidePart('display', wk('aide-display'), aideDisplayHtml(aideDisplayWithWaits(aide.display, aideCurrentWaits()), { news: aideNews(), ctx: { doc: work.doc } }));
    // What the person has now seen: a wait that comes after this is what the count says grew.
    if (work.doc) aide.seen = new Set(aideCurrentWaits().map(w => w.key));
  }
  renderAidePin();
  renderAideBack();
}

/* The row at the top of the list. Only its words and marks change, so it keeps the keyboard focus across a poll. */
function renderAidePin() {
  const pin = wk('aide-pin');
  if (!pin) return;
  const waits = work.doc ? aideCurrentWaits() : null;
  const sub = waits ? aidePinSub(waits) : '読み込み中…';
  const subEl = wk('aide-pin-sub');
  if (subEl.textContent !== sub) subEl.textContent = sub;
  const n = waits ? waits.length : 0;
  const count = wk('aide-pin-count');
  if (count.textContent !== String(n)) count.textContent = String(n);
  count.classList.toggle('zero', n === 0);
  const current = view === 'work' && !nav.task;
  if (current) pin.setAttribute('aria-current', 'true'); else pin.removeAttribute('aria-current');
}

/* Over a row the aide sent the person to: the way back. Over any row: how many waits came while they were away from the aide. */
function renderAideBack() {
  const on = !nav.task;
  const here = !on && !!aide.from && aide.from.board === nav.board && aide.from.ref === nav.task;
  // A row chosen by hand is not one the aide sent the person to.
  if (!here) aide.from = null;
  const back = wk('aide-back');
  setAidePart('back', back, here ? `${aideIcon('support_agent')}<span>副官から開きました — ${esc(aide.from.title)}</span><button type="button" class="aide-back-btn" data-aide-back>${aideIcon('arrow_back')}<span>副官に戻る</span></button>` : '');
  back.hidden = !here;
  const n = !on && work.doc ? aidePeekCount(aide.seen, aideCurrentWaits()) : 0;
  const peek = wk('aide-peek');
  setAidePart('peek', peek, n ? `${aideIcon('support_agent')}<span>副官 · 待ちが ${n} 件増えました</span>` : '');
  peek.hidden = !n;
}

/* ── going to the aide, and asking it ── */

/* The pane in the middle, whatever was selected. `start` is the person asking for it to run; `focus` gives its terminal the keyboard. */
function aideShow({ start = false, focus = true } = {}) {
  if (nav.task) go({ board: 'all', view: 'work', task: null, pane: 'detail' });
  aideTerm.focus = focus;
  if (start) aide.wantOpen = true;
  renderAide();
  refreshAide(true);
  if (focus && aideTerm.term) aideTerm.term.focus();
}

/* Typed into the aide's prompt, and sent, once it can take it. */
function aideAsk(text) {
  const v = aide.view;
  if (v && !v.startable && v.state !== 'running' && v.state !== 'starting') return note('副官に聞く', true, AIDE_NOT_STARTABLE);
  aide.pendingAsk = aideCleanText(text);
  aide.askAfter = aide.reads;
  // Not typed from the last poll's state: the aide may have stopped on a question since. aideShow reads it again.
  aideShow({ start: true });
}

/* Only once the agent has been heard from: its hooks run after the folder-trust question, so an Enter queued before that
   could never answer it. Nor while it waits on the person (a permission or a question): the Enter would pick an answer. */
function aideFlushAsk() {
  const v = aide.view;
  const handle = aideTerm.term;
  if (!aide.pendingAsk || aide.readOk <= aide.askAfter || v?.state !== 'running' || !v.agentSession || v.agentSession.status === 'waiting' || !handle || aideTerm.ended != null) return;
  if (!handle.type(aide.pendingAsk)) return;
  aide.pendingAsk = null;
  // Enter in a chunk of its own: the prompt takes text that arrives with a line break as a paste and keeps the break in it.
  setTimeout(() => { if (aideTerm.term === handle) handle.type('\r'); }, 80);
  handle.focus();
  renderAide();
}

/* A chip pressed just after the terminal mounted finds its socket still connecting: the text waits for it (a few seconds at
   most) rather than being dropped. */
function aideTypeScope(text, tries = 15) {
  const handle = aideTerm.term;
  if (!handle || aideTerm.ended != null) return note('対象', true, '副官のターミナルが開いていません');
  if (handle.type(text)) return handle.focus();
  if (tries <= 0) return note('対象', true, '副官のターミナルにつながりませんでした');
  setTimeout(() => { if (aideTerm.term === handle) aideTypeScope(text, tries - 1); }, 200);
}

/* A row of the list, opened from the aide, with the way back. The one entry for what shows a session (「ターミナルを見る」). */
function aideOpenRow(key) {
  const { rows, turns } = workListRows();
  const r = [...rows, ...turns].find(x => x.key === key) || work.hubsByKey?.get(key);
  if (!r) return note('ターミナルを見る', true, 'その仕事は、いまの一覧にありません');
  aide.from = { board: r.board, ref: r.ref, title: aideRowTitle(r) };
  selectWorkRow(r);
}

/* ── events ── */

wk('aide-pin').addEventListener('click', () => aideShow({ start: true }));

wk('wk-term').addEventListener('click', e => {
  let b;
  if (e.target.closest('[data-aide-back], #aide-peek')) return aideShow();
  if (e.target.closest('[data-aide-start]')) return aideShow({ start: true });
  if (e.target.closest('[data-aide-reconnect]')) {
    aideTerm.reconnect = true;
    aideTerm.focus = true;
    return renderAide();
  }
  if (e.target.closest('[data-aide-ask-cancel]')) {
    aide.pendingAsk = null;
    return renderAide();
  }
  if ((b = e.target.closest('[data-aide-scope]'))) {
    const scope = b.dataset.aideScope;
    return aideTypeScope(scope === '#' ? '#' : aideScopeText(scope));
  }
  if ((b = e.target.closest('[data-aide-row]'))) return aideOpenRow(b.dataset.aideRow);
  if (e.target.closest('[data-aide-ask-new]')) {
    const news = aideNews();
    if (!news) return;
    return aideAsk(news.count === 1 ? aideAskRowText(news.first) : AIDE_ASK_ALL);
  }
  if (e.target.closest('[data-aide-reset]')) {
    if (aide.resetting) return;
    // A reset in the middle of a turn throws that turn away: asked first.
    if (aideStateOf(aide.view).busy) {
      const dialog = wk('aide-reset-dialog');
      dialog.returnValue = '';
      return dialog.showModal();
    }
    resetAide();
  }
});

wk('aide-reset-dialog').addEventListener('close', e => {
  if (e.target.returnValue === 'reset') resetAide();
});
