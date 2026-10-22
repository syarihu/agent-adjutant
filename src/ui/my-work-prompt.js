/* ── Answering a prompt waiting on a session's terminal (permission approval or numbered choice) ──
   A session (Claude Code or the Antigravity CLI agy) can ask the person directly in the terminal without
   opening a gate. When the server reads such a prompt on the pane, this module displays it in the TODO
   list and over the active terminal, allowing one-tap answering by sending the chosen option's key. */

const workPrompt = { busy: new Set() };
const workPromptKey = (board, sid) => `${board}/${sid}`;

/* Reads the prompt of an entry's session, or null if there is none, if the session is not present, or if
   a gate waits on it. A gate takes priority: a gated session never shows a prompt in TODO or overlays. */
function workPromptOf(e, { isDone = () => false } = {}) {
  const p = e?.session?.prompt;
  if (!p) return null;
  if (!e.session.present) return null;
  if (workEntryGated(e, isDone)) return null;
  return p;
}

/* Words the short summary for the row or card. For a permission prompt, words "<request> を実行していい？"
   using the ledger's request (or two detail lines joined), falling back to "許可を求めています". For a choice
   prompt, words the question line, falling back to "ターミナルで聞かれています". */
function workPromptText(p, s = null, titled = false) {
  const title = titled && s ? (s.title || s.taskTitle || s.id) : '';
  let main = '';
  if (p?.kind === 'permission') {
    const req = s?.agentSession?.request && typeof isQuestion === 'function' && !isQuestion(s)
      ? s.agentSession.request
      : (p?.detail?.length >= 2 ? `${p.detail[0]}: ${p.detail[1]}` : null);
    main = req ? `${req} を実行していい？` : '許可を求めています';
  } else {
    main = p?.question ? p.question : 'ターミナルで聞かれています';
  }
  return title ? `${title}: ${main}` : main;
}

/* Converts an option to a concise button label. For permission prompts, standard approval options are mapped
   to fixed Japanese labels; for other options, labels longer than 24 characters are cut to 24 and end with … */
function promptButtonLabel(opt, kind, idx, total, hasAlwaysAllow = false) {
  const raw = (opt?.label || '').trim();
  if (kind === 'permission') {
    const lower = raw.toLowerCase();
    if (idx === 0 && lower.startsWith('yes')) {
      return '許可';
    }
    if (idx === total - 1 && lower.startsWith('no')) {
      return '断る';
    }
    if (!hasAlwaysAllow && (lower.includes('always allow') || lower.includes("don't ask again") || lower.includes('allow all'))) {
      return '今後は聞かない';
    }
  }
  return raw.length > 24 ? raw.slice(0, 24) + '…' : raw;
}

/* The prompt row as a quick-question shape: icon, question text, a small fact line, and answer buttons.
   When options could not be read reliably or require typing, a single button to answer in terminal is drawn. */
function workPromptHtml(p, opts = {}) {
  const { sessionId = '', boardSlug = '', session = null, titled = false } = opts;
  const key = workPromptKey(boardSlug, sessionId);
  const busy = opts.busy || workPrompt.busy.has(key);
  const icon = 'front_hand';
  const text = workPromptText(p, session, titled);

  const hasOptions = p.options && p.options.length > 0;
  const factText = hasOptions ? 'ターミナルで聞かれています' : '画面から選択肢を読み取れませんでした';
  const factHtml = `<div class="qc-chips"><span class="qc-chip">${esc(factText)}</span></div>`;

  let buttons = '';
  if (hasOptions) {
    let sawAlways = false;
    buttons = p.options.map((opt, i) => {
      const hasAlways = sawAlways;
      const label = promptButtonLabel(opt, p.kind, i, p.options.length, hasAlways);
      if (label === '今後は聞かない') sawAlways = true;
      const cls = i === 0 ? 'btn-m3-primary' : 'btn-m3-tonal';
      return `<button type="button" class="${cls} qc-quick-act" data-wk-prompt-key="${esc(opt.key)}" data-wk-prompt-id="${esc(p.id)}" data-wk-prompt-session="${esc(sessionId)}" data-wk-prompt-board="${esc(boardSlug)}" title="${esc(opt.label)}"${busy ? ' disabled' : ''}>${esc(label)}</button>`;
    }).join('');
  } else {
    buttons = `<button type="button" class="btn-m3-tonal qc-quick-act" data-wk-prompt-terminal="${esc(sessionId)}" data-wk-prompt-board="${esc(boardSlug)}"${busy ? ' disabled' : ''}>ターミナルで答える</button>`;
  }

  return `<div class="qc-quick wk-prompt" data-wk-prompt-box="${esc(key)}">${qcIcon(icon)}<b class="qc-quick-text">${esc(text)}</b>${factHtml}<span class="qc-spacer"></span>${buttons}</div>`;
}

/* A prompt waiting on a session as its own TODO row: question, buttons, and repository origin. */
function todoPromptRowHtml(item, busy = false) {
  const promptHtml = workPromptHtml(item.prompt, {
    sessionId: item.sessionId,
    boardSlug: item.board,
    session: { ...item.session, title: item.title },
    titled: true,
    busy,
  });
  return `<div class="todo-quick todo-prompt" data-todo-key="${esc(item.key)}">${promptHtml}<div class="todo-from">${esc(item.nwo)}</div></div>`;
}

/* Sends the selected option key to the session pane via POST /api/sessions/<id>/prompt.
   Disables buttons immediately and refreshes on 200 or 409 (question gone). */
async function workPromptPress(board, sid, id, key) {
  const k = workPromptKey(board, sid);
  if (workPrompt.busy.has(k)) return;
  workPrompt.busy.add(k);
  if (typeof renderWorkTerm === 'function') renderWorkTerm();
  if (typeof todoFollow === 'function') todoFollow(true);
  const base = board ? `/b/${board}` : '';
  try {
    const data = await boardApi(base, `/api/sessions/${enc(sid)}/prompt`, {
      method: 'POST',
      body: JSON.stringify({ id, key }),
    });
    if (data.sent) {
      if (typeof refreshWork === 'function') refreshWork(true);
      if (typeof refresh === 'function') refresh();
    }
  } catch (e) {
    if (e.status === 409) {
      note('答える', true, 'この質問は、もう待っていません');
      if (typeof refreshWork === 'function') refreshWork(true);
      if (typeof refresh === 'function') refresh();
    } else {
      note('答える', true, e.message);
    }
  } finally {
    workPrompt.busy.delete(k);
    if (typeof renderWorkTerm === 'function') renderWorkTerm();
    if (typeof todoFollow === 'function') todoFollow(true);
  }
}

if (typeof module !== 'undefined') {
  module.exports = {
    workPrompt,
    workPromptKey,
    workPromptOf,
    workPromptText,
    promptButtonLabel,
    workPromptHtml,
    todoPromptRowHtml,
    workPromptPress,
  };
}
