/* ── Over the middle terminal in 「いまの仕事」: the page's half (the markup is work-term.js) ─────────────────────────────────────
   Draws what slides down over the terminal's top (the questions of what is selected), the two floating buttons, and the block over a
   stopped hub; wires the heading's buttons; and opens the dialogs behind them: 「最後のメッセージ」, 「書いて送る」 and a hub's
   detail. All of it is laid over the terminal and none of it takes room from it, so a question coming or going never resizes the
   terminal. Drawing never moves the focus: only a click does. Loaded after my-work.js, whose `renderWorkTerm` calls it. */

/* The start check folded to its one line, by `gateKey`; kept while the page is. A form starts open. */
const workAskFolded = new Set();
/* A redraw of the questions waits while a comment box in them has the focus (checklist: no redraw while typing). */
let workAskHeld = false;
/* What the held draw was for (`workAskSubject`): a hold is dropped when the selection moved, or the old question would stay over the new terminal. */
let workAskDrawnFor = null;
const workAskSubject = () => `${nav.board}/${nav.task}`;
/* What was written in 「書いて送る」 and not sent, by `${board}/${session id}`; kept while the page is. A page reload does not keep it. */
const workSendDrafts = new Map();

const workAskEl = () => wk('wk-term-ask');
const workCommentFocused = () => !!document.activeElement?.matches?.('#wk-term-ask .gate-comment');

/* ── the questions ── */

/* The gates that slide down for what is selected: a task's own; a hub's (the page's own hub: another board's are not read here) or a
   session's with no task, which no task card shows. */
function workTermAskGates(task, own, hub) {
  const base = { gates: state.gates || [], taskOfGate, answered: answeredGates, keyOf: gateKey };
  if (task) return workTermGatesOf({ ...base, task });
  const sessGates = hub ? (hubOther(hub) ? [] : sessGatesOf(hubSessionOf(hub)))
    : own?.kind === 'worker' && boardOfSession(own).own ? sessGatesOf(own) : [];
  return workTermGatesOf({ ...base, sessGates, hub: !!hub });
}

/* Drawn as the stage's top layer; the same markup is not drawn twice, and the slide plays only when it comes in from nothing. */
function drawWorkAsk(html) {
  const el = workAskEl();
  if (!el) return;
  if (workTermPart.ask !== undefined && workTermPart.ask !== html && workCommentFocused() && workAskDrawnFor === workAskSubject()) {
    workAskHeld = true;
    return;
  }
  workAskHeld = false;
  workAskDrawnFor = workAskSubject();
  const arriving = !workTermPart.ask && !!html;
  // A redraw replaces the button that was pressed: the focus goes back to the fold button of the same question.
  const foldKey = el.contains(document.activeElement) ? document.activeElement.closest?.('[data-wk-fold]')?.dataset.wkFold : null;
  setWorkTermPart('ask', el, html);
  if (foldKey != null && !el.contains(document.activeElement)) {
    [...el.querySelectorAll('[data-wk-fold]')].find(b => b.dataset.wkFold === foldKey)?.focus();
  }
  if (arriving) {
    el.classList.remove('slide');
    void el.offsetWidth;
    el.classList.add('slide');
  }
}

/* Why 「書いて送る」 is not possible just now, or ''. */
const workSendWhy = () => workTerm.ended != null ? 'ターミナルは切れています。「再接続」を押してください' : '';

/* The two buttons over the terminal's bottom right: only while the terminal itself is on screen (the placeholder covers it
   otherwise). 最後のメッセージ is there while the running agent has said something. */
function drawWorkFabs(own, hub, reading) {
  const s = hub ? hubSessionOf(hub) : own;
  const a = s?.present ? s.agentSession : null;
  const mins = a?.lastMessageAt != null && state.now != null ? minutesSince(a.lastMessageAt, state.now) : null;
  const html = workTerm.term && !reading && s?.present
    ? workTermFabsHtml({ hasLast: !!a?.lastMessage, lastAge: mins != null ? agoLabel(mins) : '', sendWhy: workSendWhy() }) : '';
  setWorkTermPart('fabs', wk('wk-term-fabs'), html);
}

/* Called by `renderWorkTerm` once the head, the bar and the terminal are in place. */
function renderWorkTermOverlays({ own, task, hub, reading, stopped = false }) {
  const gates = reading ? [] : workTermAskGates(task, own, hub);
  // Over a stopped hub the start check is one line, so the block with 「hub を起動」 under it is in reach.
  drawWorkAsk(workTermAskHtml(gates, {
    ctxOf: questionCtxOf, keyOf: gateKey, hub: !!hub, folded: workAskFolded, draftOf: key => questionDrafts.get(key) || '', foldAll: stopped,
  }));
  // What the placeholder says starts below what slides down over it.
  const ph = wk('wk-term-ph');
  const askBox = workAskEl().getBoundingClientRect();
  ph.style.paddingTop = !ph.hidden && askBox.height ? `${Math.ceil(askBox.bottom - ph.getBoundingClientRect().top + 12)}px` : '';
  drawWorkFabs(own, hub, reading);
  // Only the count is announced: the questions themselves are read where they are, and a redraw must not read the form out again.
  setWorkTermPart('askStatus', wk('wk-term-ask-status'), gates.length ? `確認待ちが ${gates.length} 件あります` : '');
}

workAskEl().addEventListener('click', e => {
  const fold = e.target.closest('[data-wk-fold]');
  if (fold) {
    const key = fold.dataset.wkFold;
    if (workAskFolded.has(key)) workAskFolded.delete(key); else workAskFolded.add(key);
    return renderWorkTerm();
  }
  const gate = e.target.closest('[data-qc-open-gate]');
  if (gate) {
    const g = questionLive(gate.dataset.qcOpenGate);
    return g ? openQuestionDialog(g, { opener: gate }) : note('開く', true, 'この質問は、もう待っていません');
  }
  const open = e.target.closest('[data-qc-open]');
  if (open) return questionOpenLayer(open.dataset.qcOpen, open);
  const b = e.target.closest('[data-qc-gate] .pick[data-choice], [data-qc-gate] [data-act], [data-qc-ide]');
  if (b && !b.disabled) questionAct(b);
});
workAskEl().addEventListener('input', e => {
  const box = e.target.closest?.('.gate-comment');
  const key = box?.closest('[data-qc-gate]')?.dataset.qcGate;
  if (!key) return;
  if (box.value) questionDrafts.set(key, box.value); else questionDrafts.delete(key);
});
workAskEl().addEventListener('focusout', e => {
  if (!workAskHeld || !e.target.matches?.('.gate-comment')) return;
  // Focus moving to a button of the questions: drawing now would replace the button between its mousedown and its click and lose
  // the click. That click (or the next poll) redraws instead.
  if (e.relatedTarget && workAskEl().contains(e.relatedTarget)) return;
  setTimeout(() => {
    if (!workAskHeld || workCommentFocused()) return;
    workAskHeld = false;
    renderWorkTerm();
  });
});

/* ── the heading's buttons ── */

/* The session of the row selected, as the board lists it; a stopped hub with none has its record's. */
function workTermSession() {
  const sel = workSelected();
  const id = sel?.session?.id || (isSessRef(nav.task) ? sessIdOfRef(nav.task) : null);
  if (!id) return null;
  const hub = sel?.session?.kind === 'hub' ? (state.hubs || []).find(x => x.id === id) : null;
  return (state.sessions || []).find(x => x.id === id) || (hub ? hubSessionOf(hub) : null);
}

wk('wk-term-head').addEventListener('click', e => {
  const b = e.target.closest('[data-wk-term-act]');
  if (!b || b.disabled) return;
  const act = b.dataset.wkTermAct;
  const s = workTermSession();
  if (act === 'detail') {
    const task = isTaskRef(nav.task) ? taskById(nav.task) : null;
    return task ? openCardDialogTask(task, b, null) : undefined;
  }
  if (!s) return;
  if (act === 'hubinfo') {
    const hub = (state.hubs || []).find(h => h.id === s.id);
    return hub ? openWorkHubInfo(hub, b) : undefined;
  }
  // The same two the session's menu and bar have, with the same checks.
  if (act === 'ide' && linkedWorktree(s)) return runSessionAction('ide', s);
  if (act === 'close' && s.kind === 'worker' && linkedWorktree(s)) return runSessionAction('close', s);
});

/* ── the floating buttons ── */

wk('wk-term-fabs').addEventListener('click', e => {
  const b = e.target.closest('[data-wk-fab]');
  if (!b || b.disabled) return;
  if (b.dataset.wkFab === 'lastmsg') return openWorkLastMessage(b);
  if (b.dataset.wkFab === 'send') openWorkSend(b);
});

/* ── 最後のメッセージ ── */

let workLastToken = 0;

/* The agent's last message, as Markdown, in a dialog. The state carries the first part of it; when that is cut, the whole is read, and
   applied only to the dialog it was asked for, for the message it was asked about. */
function openWorkLastMessage(opener) {
  const s = workTermSession();
  const a = s?.present ? s.agentSession : null;
  if (!a?.lastMessage) return note('最後のメッセージ', true, 'このセッションはまだ何も話していません');
  const board = nav.board;
  const key = `lastmsg:${board}/${s.id}`;
  if (dialogStack.has(key)) return;
  if (dialogStack.isOpen()) dialogStack.closeAll();
  const mins = a.lastMessageAt != null && state.now != null ? minutesSince(a.lastMessageAt, state.now) : null;
  const title = wk('wk-term-head').querySelector('.wk-term-title')?.textContent || s.id;
  const cut = lastMessageCut(a.lastMessage);
  const noteHtml = text => `<div class="tp-muted lastmsg-note" role="status">${esc(text)}</div>`;
  const token = ++workLastToken;
  dialogStack.push({
    key, title: `最後のメッセージ — ${title}${mins != null ? `（${agoLabel(mins)}）` : ''}`, opener,
    body: `<div data-wk-lastmsg>${cut ? noteHtml('全文を読み込んでいます…') : ''}<div class="body">${md(a.lastMessage)}</div></div>`,
  });
  if (!cut) return;
  const fill = html => {
    // Not for another dialog, another message, or a board the page has left since.
    if (token !== workLastToken || !dialogStack.has(key) || nav.board !== board) return;
    const el = dialogStack.layerEl(key)?.querySelector('[data-wk-lastmsg]');
    if (el) el.innerHTML = html;
  };
  api(`/api/sessions/${enc(s.id)}/message`).then(data => {
    if (data?.sessionId !== a.sessionId || data.at !== a.lastMessageAt) {
      return fill(noteHtml('このあとに別のメッセージが届いたため、全文は読み込んでいません。表示しているのは、開いたときの先頭の部分です') + `<div class="body">${md(a.lastMessage)}</div>`);
    }
    const partial = data.whole === false ? noteHtml(`全文は残っていないため、先頭の ${LAST_MESSAGE_SHOWN_CHARS} 文字までを表示しています`) : '';
    fill(`${partial}<div class="body">${md(data.text || a.lastMessage)}</div>`);
  }).catch(err => {
    fill(noteHtml(`全文を読み込めませんでした: ${err.message}（先頭の ${LAST_MESSAGE_SHOWN_CHARS} 文字までを表示しています）`) + `<div class="body">${md(a.lastMessage)}</div>`);
  });
}

/* ── 書いて送る ── */

/* The box that is open: where it sends (the board and the session as they were when it opened), and whether a send went in. */
let workSendTarget = null;
let workSending = false;

function openWorkSend(opener) {
  if (!workTerm.term || !workTerm.sessionId) return note('書いて送る', true, 'ターミナルは開いていません');
  const board = workTerm.board;
  const sessionId = workTerm.sessionId;
  const draftKey = `${board}/${sessionId}`;
  const key = `send:${draftKey}`;
  if (dialogStack.has(key)) return;
  if (dialogStack.isOpen()) dialogStack.closeAll();
  const title = wk('wk-term-head').querySelector('.wk-term-title')?.textContent || sessionId;
  const { body, foot } = workSendParts({ draft: workSendDrafts.get(draftKey) || '', enter: prefs.sendEnter !== false });
  workSendTarget = { key, board, sessionId, draftKey, sent: false };
  dialogStack.push({
    key, title: `ターミナルに送る — ${title}`, body, foot: `<div class="wk-send-foot">${foot}</div>`, opener,
    onClose: layer => {
      // What is written stays for the next opening, unless it went in.
      const typed = dialogStack.layerEl(layer.key)?.querySelector('.wk-send-text')?.value;
      if (typed != null && !workSendTarget?.sent) typed ? workSendDrafts.set(draftKey, typed) : workSendDrafts.delete(draftKey);
      if (workSendTarget?.key === key) workSendTarget = null;
    },
  });
  dialogStack.layerEl(key)?.querySelector('.wk-send-text')?.focus();
}

/* Sends what is written as one paste, and presses Enter after it when asked. It goes to the terminal the box was opened on, and only
   while that is the one in the middle and open: otherwise nothing is sent and the draft stays. A refused or failed send keeps the draft. */
async function workSend(layerEl, withEnter) {
  const target = workSendTarget;
  if (!target || workSending) return;
  const box = layerEl.querySelector('.wk-send-text');
  const text = workSendBody(box.value);
  if (!text.trim()) return note('ターミナルに送る', true, '送る内容が空です');
  if (workSendTooLong(text)) return note('ターミナルに送る', true, `長すぎるため送りませんでした（上限は ${WORK_SEND_MAX_BYTES / 1024} KiB）。下書きは残してあります`);
  const term = workTerm.term;
  if (!term || workTerm.board !== target.board || workTerm.sessionId !== target.sessionId) {
    return note('ターミナルに送る', true, '開いたときのセッションのターミナルではなくなったため、送りませんでした。下書きは残してあります');
  }
  const buttons = [...layerEl.querySelectorAll('.qd-foot button')].filter(x => !x.disabled);
  buttons.forEach(x => { x.disabled = true; });
  // Read-only while it goes: what is typed meanwhile would be kept as a draft of text that was sent.
  box.readOnly = true;
  workSending = true;
  let done = false;
  try {
    if (!term.paste(text)) {
      return note('ターミナルに送る', true, 'ターミナルがつながっていないため、送れませんでした。下書きは残してあります');
    }
    target.sent = true;
    workSendDrafts.delete(target.draftKey);
    if (withEnter) {
      await new Promise(resolve => setTimeout(resolve, WORK_SEND_ENTER_MS));
      if (workTerm.board !== target.board || workTerm.sessionId !== target.sessionId) note('ターミナルに送る', true, '貼り付けは済みましたが、表示しているセッションが変わったため、Enter は押していません');
      else if (workTerm.term !== term) note('ターミナルに送る', true, '貼り付けは済みましたが、ターミナルがつなぎ直されたため、Enter は押していません。必要ならターミナルで押してください');
      else if (!term.enter()) note('ターミナルに送る', true, '貼り付けは済みましたが、Enter を送れませんでした。ターミナルで押してください');
    }
    done = true;
  } catch (err) {
    // A throw from the terminal: nothing is known to have gone in unless `sent` says so, and the draft stays otherwise.
    note('ターミナルに送る', true, `${err?.message || err}${target.sent ? '' : '。下書きは残してあります'}`);
    done = target.sent;
  } finally {
    workSending = false;
    box.readOnly = false;
    if (!done) buttons.forEach(x => { if (x.isConnected) x.disabled = false; });
  }
  // Not another dialog that was opened meanwhile.
  if (dialogStack.has(target.key)) dialogStack.closeAll();
  // Only put in: the person reads it in the input box and presses Enter.
  if (!withEnter && workTerm.term === term) term.focus();
}

/* `workSend` from a listener: a failure is said, not left as an unhandled rejection. */
const workSendSafely = (layer, withEnter) => workSend(layer, withEnter)
  .catch(err => note('ターミナルに送る', true, `${err?.message || err}。下書きは残してあります`));

const workSendLayer = e => {
  const layer = e.target.closest?.('.qd-layer');
  return layer?.dataset.qdKey?.startsWith('send:') ? layer : null;
};
questionDialogEl().addEventListener('click', e => {
  const layer = workSendLayer(e);
  const b = layer && e.target.closest('[data-wk-send]');
  if (b && !b.disabled) workSendSafely(layer, b.dataset.wkSend === 'send' && layer.querySelector('[data-wk-send-enter]').checked);
});
questionDialogEl().addEventListener('keydown', e => {
  const layer = workSendLayer(e);
  // An Enter that confirms IME composition sends nothing (keyCode 229 where compositionend came first).
  if (!layer || !e.target.matches('.wk-send-text') || !(e.metaKey || e.ctrlKey) || e.key !== 'Enter' || e.isComposing || e.keyCode === 229) return;
  e.preventDefault();
  workSendSafely(layer, layer.querySelector('[data-wk-send-enter]').checked);
});
questionDialogEl().addEventListener('input', e => {
  const layer = workSendLayer(e);
  if (!layer || !e.target.matches('.wk-send-text') || !workSendTarget || workSendTarget.sent) return;
  if (e.target.value) workSendDrafts.set(workSendTarget.draftKey, e.target.value); else workSendDrafts.delete(workSendTarget.draftKey);
});
questionDialogEl().addEventListener('change', e => {
  if (!workSendLayer(e) || !e.target.matches('[data-wk-send-enter]')) return;
  prefs.sendEnter = e.target.checked;
  savePrefs();
});

/* ── a hub's detail ── */

/* The hub's panel, which the work view does not have a column for: the same detail, in a dialog. */
/* The banner that names the gate a hub is waiting on and opens it, when that gate is not one of the lines over the terminal (it has a
   task card, or is on another board): the panel's own rule (`sessJudgeOf`). */
function workHubWaitingHtml(h, s) {
  const open = workTermAskGates(null, null, h);
  return s.waiting && !open.some(g => g.id === s.waiting.id) ? sessGateHtml(s) : '';
}

const workHubInfoHtml = (h, s) => workHubWaitingHtml(h, s) + (h.titleState ? `<div class="hub-title-tools">${hubTitleMarkHtml(h)}${hubTitleReadButtonHtml(h, hubPanelWakeBase(h))}${hubTitleWhyHtml(h)}</div>` : '')
  // The button that expands a card is the panel's own; nothing in a dialog answers it.
  + hubDetailHtml(h, s).replace(/<button[^>]*data-expand-open[\s\S]*?<\/button>/g, '');

/* The hub dialog that is open: { key, board, id, html (as drawn), gone }. */
let workHubInfo = null;

function openWorkHubInfo(hub, opener) {
  const key = `hubinfo:${nav.board}/${hub.id}`;
  if (dialogStack.has(key)) return;
  if (dialogStack.isOpen()) dialogStack.closeAll();
  const html = workHubInfoHtml(hub, hubSessionOf(hub));
  workHubInfo = { key, board: nav.board, id: hub.id, html, gone: false };
  dialogStack.push({
    key, title: `hub の詳細 — ${hubLabel(hub)}`, body: html, opener,
    onClose: layer => { if (workHubInfo?.key === layer.key) workHubInfo = null; },
  });
}

/* The dialog follows the page's state: its body is drawn again only when its markup changed and the focus is not in it, keeping where
   it was scrolled to and which sections were open. A hub the board no longer lists leaves it as it was, with a notice. */
function workTermDialogsFollow() {
  const info = workHubInfo;
  if (!info || !dialogStack.isOpen() || info.gone || nav.board !== info.board) return;
  const layer = dialogStack.layerEl(info.key);
  if (!layer) return;
  const hub = (state.hubs || []).find(h => h.id === info.id);
  if (!hub) {
    if (state.now == null) return;
    info.gone = true;
    layer.querySelectorAll('.qd-body button').forEach(x => { x.disabled = true; });
    layer.querySelector('.qd-head')?.insertAdjacentHTML('afterend', '<p class="qd-gone" role="status">この hub は一覧にありません</p>');
    return;
  }
  const html = workHubInfoHtml(hub, hubSessionOf(hub));
  const body = layer.querySelector('.qd-body');
  if (html === info.html || (document.activeElement !== body && body.contains(document.activeElement))) return;
  const top = body.scrollTop;
  const open = [...body.querySelectorAll('details')].map(d => d.open);
  body.innerHTML = html;
  body.scrollTop = top;
  body.querySelectorAll('details').forEach((d, i) => { if (open[i] != null) d.open = open[i]; });
  info.html = html;
}

/* A click in the hub dialog is the hub panel's click: the panel's own handler, for the hub the dialog was opened on. A dialog opened by
   an action is left first, for a dialog does not open over another one. */
const WORK_HUB_OPENS_DIALOG = ['stop', 'close', 'reset', 'restart', 'add-session', 'add-parent-hub'];
questionDialogEl().addEventListener('click', e => {
  const layer = e.target.closest?.('.qd-layer');
  if (!layer?.dataset.qdKey?.startsWith('hubinfo:') || !workHubInfo) return;
  const info = workHubInfo;
  if (info.board !== nav.board) return note('hub の詳細', true, '別のボードに移ったため、ここからは操作できません');
  const hub = (state.hubs || []).find(h => h.id === info.id);
  if (!hub) return note('hub の詳細', true, 'この hub は一覧にありません');
  const b = e.target.closest('[data-tp-hub]');
  const act = b && !b.disabled ? b.dataset.tpHub : '';
  // The gate's own place is in the page behind the dialog.
  if (e.target.closest('[data-tp-sess-gate]')) dialogStack.closeAll();
  else if (WORK_HUB_OPENS_DIALOG.includes(act)) dialogStack.closeAll();
  // The same start as the stopped block's, with the same state while it is asked for.
  else if (act === 'start') return workStartHub(hub);
  const idle = e.target.closest('[data-tp-worktree]');
  if (idle) {
    dialogStack.closeAll();
    return openIdleWorktree(idle.dataset.tpWorktree);
  }
  const queued = e.target.closest('[data-tp-task]');
  if (queued) {
    dialogStack.closeAll();
    return go({ board: nav.board, view: 'work', task: queued.dataset.tpTask, pane: 'detail' });
  }
  hubPanelClick(e, hub);
});

/* ── a stopped hub ── */

/* The hubs whose start was asked for from here and has not answered, by `${board}/${hub id}`. The start request answers once the hub is up,
   so until then the hub is coming up, not stopped: its block says so and has no button to press twice. Page-local: the markers a
   reset sets (`hubStartingNow`) are shared by every repository hub. */
const workHubStarting = new Set();
const workHubStartKey = hub => `${nav.board}/${hub.id}`;
const workHubStartPending = hub => !!hub && workHubStarting.has(workHubStartKey(hub));

/* 「hub を起動」 from the stopped block or the hub's dialog. A failure is said by the action itself, and the block comes back. */
async function workStartHub(hub) {
  const key = workHubStartKey(hub);
  if (workHubStarting.has(key)) return;
  // A new terminal is mounted once its window exists (syncTermSlot).
  workTerm.reconnect = true;
  workHubStarting.add(key);
  renderWorkTerm();
  try {
    await runSessionAction('hub-start', hubSessionOf(hub));
  } finally {
    workHubStarting.delete(key);
    renderWorkTerm();
  }
}

/* The block over a stopped hub's terminal (`workStoppedHubShown` says when). */
function workHubStoppedBlock(hub) {
  const s = hubSessionOf(hub);
  return workHubStoppedHtml({ since: sinceLabel(hubRowOf(hub)?.hubLastAlive), unread: hub.unseen || 0, act: hubActionOf(s) });
}
