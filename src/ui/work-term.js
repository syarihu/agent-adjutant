/* ── Over the middle terminal in 「いまの仕事」: the pure half (work-term-view.js draws and wires it) ─────────────────────────────
   What is laid over the selected session's terminal, and what the heading says, as functions of what they are given: which gates
   slide down over the top of the terminal, how they read (a line each, a quick row, and for a hub's start check a form that folds
   to one line), the buttons at the heading's right end, the two floating buttons, the block over a stopped hub, and the box that
   sends text into the terminal. They touch neither the page nor the network, so `src/ui/tests/work-term.test.js` runs them under
   `node --test`. Loaded after question-card.js (`questionStripHtml`, `questionCardArticleHtml`, `qcIcon`), decide.js (`BUTTONS`,
   `kindOf`) and session-actions.js (`actionButtonHtml`), and before my-work.js. Everything that is drawn over the terminal is
   markup of its own, never a child of the terminal's host: the host's size is the terminal's, and a question coming or going must
   not resize it. */

/* The pause between the pasted text and the Enter that follows it: the agent reads a paste as one event, and an Enter that
   arrives in the same chunk can be taken as a part of it. */
const WORK_SEND_ENTER_MS = 150;

/* The text of a send, as it may go into the terminal: the control characters out, but the tab and the line break, which a paste
   carries. An Escape in it would end the bracketed paste early, and what follows would be typed as keys. Both the C0 and the C1
   (U+0080 to U+009F) ranges, and DEL. A CR is dropped, so a CRLF is the line break alone. */
const workSendableText = text => String(text ?? '').replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, '');

/* What is sent: the text as it may go in, with the lines around it that hold nothing dropped (a draft that ends in a line break would press Enter). */
const workSendBody = text => workSendableText(text).replace(/^(?:[ \t]*\n)+/, '').replace(/(?:\n[ \t]*)+$/, '');

/* The most a send carries, in UTF-8 bytes. The server drops a connection that sends a message over 1 MiB (`MAX_MESSAGE` in
   src/infra/ws.rs), and xterm sends a paste as one message, so a send is kept well under it. */
const WORK_SEND_MAX_BYTES = 512 * 1024;
const workSendTooLong = text => new TextEncoder().encode(text).length > WORK_SEND_MAX_BYTES;

/* The gates that slide down over the terminal of what is selected. For a task, the ones it owns, on its board and not answered
   here; for a hub, or a session with no task, `sessGates` (what it waits on, which the page reads only for a session of the board
   it is on) without the ones a task card shows (but a hub's dispatch gates, its start checks, whoever's task they name). `taskOfGate` and `keyOf` are the page's. */
function workTermGatesOf({ task = null, hub = false, gates = [], sessGates = [], taskOfGate, answered, keyOf }) {
  const open = g => g.wait !== false && !answered.has(keyOf(g));
  if (task) return gates.filter(g => g.task === task.id && open(g) && (!task._slug || g._slug === task._slug));
  // A hub's start check is its own to answer here, whether or not a task has a card for it.
  return sessGates.filter(g => open(g) && (!taskOfGate(g) || (hub && g.kind === 'dispatch')));
}

/* A hub's start check, which is a dispatch question that is not a quick one: it is a form over the terminal. */
const workIsStartCheck = (g, hub) => !!hub && g.kind === 'dispatch' && !g.quick;

/* The start check folded to one line: the kind, the question, the gate's first button and 「広げる」. Inside the gate's own
   element, so a click on the button is answered as it is anywhere else. With a comment typed in the form the button is left out,
   for it would send the answer without it. */
function workAskFoldedHtml(g, key, draft, expand = true) {
  const line = gateFocusLine(g.focus) || gateFocusLine(g.title);
  const first = !draft && (g.options || [])[0];
  const button = first && BUTTONS[first] ? BUTTONS[first](g) : '';
  return `<div class="qc-quick wk-ask-folded" data-qc-gate="${esc(key)}">${qcIcon('bolt')}<span class="qc-kind">${esc(kindOf(g.kind)[0])}</span>`
    + `<b class="qc-quick-text">${esc(line)}</b><span class="qc-spacer"></span>${button}`
    + (expand ? `<button type="button" class="btn-m3-text" data-wk-fold="${esc(key)}" aria-expanded="false">${qcIcon('expand_more')}<span>広げる</span></button>` : '') + '</div>';
}

/* The start check open: the card as one article under a bar that folds it. The card scrolls inside the sheet, and its foot stays. */
function workAskFormHtml(g, key, ctx, draft) {
  return `<section class="wk-ask-form" aria-label="hub の着手確認"><div class="wk-ask-bar">${qcIcon('bolt')}<span>hub の着手確認</span><span class="qc-spacer"></span>`
    + `<button type="button" class="btn-m3-text" data-wk-fold="${esc(key)}" aria-expanded="true">${qcIcon('expand_less')}<span>たたむ</span></button></div>`
    + `<div class="wk-ask-scroll">${questionCardArticleHtml(g, { ...ctx, key, draft })}</div></section>`;
}

/* What slides down: the start check first (a form, or its one line when its key is in `folded`), then a line for each of the rest,
   which a quick question answers where it stands and any other opens as a card. `ctxOf` gives a gate's card context, `keyOf` its
   key, `draftOf` the comment typed for a key; `foldAll` draws the start check as its one line whatever `folded` says, with no button to open it. */
function workTermAskHtml(gates, { ctxOf, keyOf, hub = false, folded = new Set(), draftOf = () => '', foldAll = false } = {}) {
  const form = (gates || []).find(g => workIsStartCheck(g, hub));
  const rest = (gates || []).filter(g => g !== form);
  let html = '';
  if (form) {
    const key = keyOf(form);
    const draft = draftOf(key) || '';
    html += foldAll || folded.has(key) ? workAskFoldedHtml(form, key, draft, !foldAll) : workAskFormHtml(form, key, ctxOf(form), draft);
  }
  return html + questionStripHtml(rest, ctxOf);
}

/* The buttons at the heading's right end. 「タスク詳細」 is for a task's row; 「IDE」 and 「セッションを閉じる」 for a worker with a
   linked worktree (`closeWhy` is why the close is not possible just now, or ''); 「hub の詳細」 for a hub. */
function workTermHeadActsHtml({ task = false, ide = false, close = false, closeWhy = '', hub = false } = {}) {
  const btn = (act, icon, label, extra = '') => `<button type="button" class="btn-m3-text" data-wk-term-act="${act}"${extra}>${qcIcon(icon)}<span>${esc(label)}</span></button>`;
  return (task ? btn('detail', 'description', 'タスク詳細') : '')
    + (hub ? btn('hubinfo', 'info', 'hub の詳細') : '')
    + (ide ? btn('ide', 'code', 'IDE') : '')
    + (close ? btn('close', 'tab_close', 'セッションを閉じる', closeWhy ? ` disabled title="${esc(closeWhy)}"` : ' title="worker のタブを閉じます（worktree は残ります）"') : '');
}

/* A hub's status in one line under its title: the pill the panel's head had, and the counts the 詳細 tab had in its first card.
   `counts` is [[label, value]]. */
function workHubStatusLineHtml(text, cls, counts = []) {
  return `<div class="wk-hub-line"><span class="m3-pill ${esc(cls)}">${esc(text)}</span>`
    + `${counts.length ? `<span class="tp-muted">${esc(counts.map(([l, v]) => `${l} ${v}`).join(' · '))}</span>` : ''}</div>`;
}

/* Whether the block over a stopped hub is shown: the hub has no process and is not on its way up. A terminal that was mounted before it
   stopped is a dead one, so the block covers it as well; nothing is shown over the terminal of a read-through. */
const workStoppedHubShown = ({ isHub = false, present = false, starting = false, reading = false } = {}) => isHub && !present && !starting && !reading;

/* Over a stopped hub's terminal: how long it has been stopped, what is waiting in its inbox, and the button that starts it
   (`act` is the hub's action, which is 「hub を起動」 unless it is a parent's hub nothing reports to any more). */
function workHubStoppedHtml({ since = '', unread = 0, act = null } = {}) {
  return `<span class="material-symbols-outlined wk-hub-stopped-icon" aria-hidden="true">pause_circle</span>`
    + `<div class="tp-ph-head">${esc(`hub は${since ? `${since}から` : ''}止まっています`)}</div>`
    + (unread > 0 ? `<div class="tp-muted">${esc(`受信箱に未読の依頼が ${unread} 件`)}</div>` : '')
    + (act ? `<div>${actionButtonHtml(act)}</div>` : '');
}

/* The two buttons over the terminal's bottom right. `lastAge` is how long ago the last message was said, '' for no message
   to show; `sendWhy` is why sending is not possible just now, or ''. */
function workTermFabsHtml({ hasLast = false, lastAge = '', sendWhy = '' } = {}) {
  const last = hasLast
    ? `<button type="button" class="wk-fab" data-wk-fab="lastmsg" aria-label="${esc(`最後のメッセージ${lastAge ? `（${lastAge}）` : ''}`)}" title="最後のメッセージ">${qcIcon('chat')}<span>最後のメッセージ</span>${lastAge ? `<span class="wk-fab-age">${esc(lastAge)}</span>` : ''}</button>` : '';
  const send = `<button type="button" class="wk-fab primary" data-wk-fab="send" aria-label="書いて送る"${sendWhy ? ` disabled title="${esc(sendWhy)}"` : ' title="書いて送る"'}>${qcIcon('edit_square')}<span>書いて送る</span></button>`;
  return last + send;
}

/* The box that sends: the body (what is written, and what the send does) and the foot (the choices, and the buttons). `draft` is
   what was written before, `enter` whether Enter is pressed after it. */
function workSendParts({ draft = '', enter = true } = {}) {
  const body = '<p class="tp-muted wk-send-hint">ターミナルの入力欄に 1 回の貼り付けとして入ります。下書きはセッションごとに残ります。</p>'
    + `<textarea class="wk-send-text" aria-label="ターミナルに送る内容" placeholder="セッションに伝えること。Markdown のまま書ける" rows="8">${esc(draft)}</textarea>`;
  const foot = `<label class="wk-send-enter"><input type="checkbox" data-wk-send-enter${enter ? ' checked' : ''}><span>送ったら Enter も押す</span></label>`
    + '<span class="tp-muted wk-send-key">⌘ Enter で送る</span><span class="qc-spacer"></span>'
    + '<button type="button" class="btn-m3-tonal" data-wk-send="paste">ターミナルに入れるだけ</button>'
    + '<button type="button" class="btn-m3-primary" data-wk-send="send">送る</button>';
  return { body, foot };
}
