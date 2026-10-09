/* What the page knows of a session: its state, the words that name it, the board it belongs to. A session opens in
   the task panel, as a task does (task-panel.js), in 「いまの仕事」 or over a board. Mostly pure (it reads `state` and
   returns). What can be done to a session is in session-actions.js. */

/* A `running` row whose tmux window has been quiet this long reads as idle (seconds; window_activity
   is the only clock there is, so this is a guess at what "quiet" means). Only a row that says
   running is read this way: with no row the window's activity says nothing, since a resize redraws it. */
const IDLE_AFTER_SECS = 60;
/* The phases at which a worker that is gone has finished rather than stopped (see stuckOf). */
const FINISHED_PHASES = ['pr', 'pr-bots', 'review', 'report'];
/* What a `permission` session is called where it is drawn from its own session. The ledger's
   `waiting` also covers other dialogs the agent asks, so it is 許可待ち only with a request to
   say what is asked; the tables below hold the neutral word, for where there is no session. */
const permissionLabel = s => isQuestion(s) ? '質問への回答待ち' : s?.agentSession?.request ? '許可待ち' : '入力待ち';
/* A question the agent asks with its AskUserQuestion tool reaches the ledger as a permission
   request whose text starts with this (src/transport/cli/hook.rs, tool_summary). */
const QUESTION_PREFIX = 'AskUserQuestion: ';
const isQuestion = s => {
  const request = s?.agentSession?.request || '';
  // Bare when the tool call carried no question text.
  return request.startsWith(QUESTION_PREFIX) || request === QUESTION_PREFIX.trim();
};
/* What the agent asks, in a sentence: a question, or the permission for a tool. Plain text. */
const requestText = s => {
  const request = s?.agentSession?.request || '';
  if (!isQuestion(s)) return `許可を求めています: ${request}`;
  const question = request.slice(QUESTION_PREFIX.length).trim();
  return question ? `質問しています: ${question}` : '質問しています';
};
const STATE_ORDER = { waiting: 0, permission: 1, stopped: 2, restarting: 2, failed: 2, idle: 3, done: 3, unknown: 3, working: 4, ended: 5, none: 6 };
const STATE_LABEL = {
  waiting: '確認待ち', permission: '入力待ち', done: '待機中', failed: 'エラー（API）', stopped: '停止', idle: '待機中（出力なし）', unknown: '状態不明', working: '作業中', ended: '終了', none: 'セッションなし',
  pending: '起動を依頼中…', restarting: '再起動しています…',
};
const STATE_ICON = {
  waiting: 'help', permission: 'front_hand', done: 'hourglass_empty', failed: 'error', stopped: 'error', idle: 'hourglass_empty', unknown: 'visibility_off', working: 'play_circle', ended: 'check_circle', none: 'remove_circle_outline', pending: 'hourglass_top', restarting: 'autorenew',
};
const STATE_PILL = {
  waiting: 'pill-warn', permission: 'pill-warn', done: 'pill-neutral', failed: 'pill-err', stopped: 'pill-err', idle: 'pill-neutral', unknown: 'pill-neutral', working: 'pill-good', ended: 'pill-blue', none: 'pill-neutral', restarting: 'pill-neutral',
};
/* A row's state in the few words it has room for. A quiet window is running too: it only has
   the neutral pill (STATE_PILL), so it is not taken for one that is writing. A session whose
   state is not known is not called running at all. */
const ROW_LABEL = { waiting: '入力待ち', permission: '入力待ち', done: '待機', failed: 'エラー', stopped: '停止', idle: '稼働', unknown: '不明', working: '稼働', ended: '終了', restarting: '再起動' };

const hubOfSession = s => s.kind === 'hub' ? s.id : s.hub;

/* The one place that decides which board a session belongs to. A worker's `hub` names a
   `hubs[]` entry, and that entry's slug is its board: `/b/<slug>` on the resident server.
   Its base is the one a request about it goes to. The terminal socket is not: it stays on this
   page's board, which lists every linked worktree's session, while a parent-task hub's slug
   may be missing from the resident server's address book. */
function boardOfSession(s) {
  const hubId = hubOfSession(s);
  const hub = (state.hubs || []).find(h => h.id === hubId) || null;
  const base = state.resident && hub?.slug ? `/b/${hub.slug}` : BASE;
  return { hubId, hub, slug: hub?.slug || null, base, own: base === BASE };
}

/* The functions below read `data`, a state: the page's own, or one repository's part of 「すべて」,
   where a hub's id is only its repository's. */
function sessionActivity(s, data = state) {
  return data.now != null && s.lastActivityAt != null && data.now - s.lastActivityAt >= IDLE_AFTER_SECS
    ? 'idle' : 'working';
}

/* The agent ledger's statuses this page knows, as its states (my-work-away.js keeps the same words). */
const AGENT_STATES = { running: 'working', waiting: 'permission', idle: 'done', done: 'done', failed: 'failed' };

/* What the agent's hooks say, as a state of this page, or null when there is no row, the ledger
   could not be read, or the status is one this page does not know. */
function agentStateOf(s) {
  const a = s.agentSession;
  if (!s.present || !a || a.error) return null;
  return Object.hasOwn(AGENT_STATES, a.status) ? AGENT_STATES[a.status] : null;
}

/* The state of a session that runs: the ledger's, except `running`, which defers to the pane. No
   hook fires when a turn is interrupted with Esc, so a row can stay `running` long after the agent
   stopped; the pane going quiet is what tells (on tmux). With no usable row it is `unknown`, not
   a guess from the pane: opening a terminal resizes it, and the redraw counts as activity. */
function ledgerState(s, data) {
  const st = agentStateOf(s);
  if (!st) return 'unknown';
  return st === 'working' ? sessionActivity(s, data) : st;
}

/* Why a running session's state is not known, as plain text, or empty when it is. Three causes,
   told apart: no row (the agent's hooks never reached the ledger), a ledger that could not be
   read, and a status word this page does not know. */
function unknownWhy(s) {
  if (!s?.present || agentStateOf(s)) return '';
  const a = s.agentSession;
  if (!a) return 'エージェントのフックから何も届いていないため、作業中か入力待ちか分かりません。adjutant が起動した Claude Code は「セッションを再起動」で、それ以外は `adjutant setup claude` / `adjutant setup codex` のあとに起動し直すとフックが届きます。';
  if (a.error) return 'エージェントの状態を読めません';
  return `エージェントの報告: ${a.status}`;
}

/* What the agent's hooks say beyond its state, as plain text (escaped where it is drawn), or
   empty when they say nothing. */
function agentText(s, data = state) {
  const a = s.agentSession;
  if (!s.present || !a) return '';
  if (a.error) return unknownWhy(s);
  const parts = [];
  if (agentStateOf(s) === 'permission' && a.request) parts.push(requestText(s));
  // What the row says it is: a `running` the pane has gone quiet on is not doing the tool.
  else if (a.activity && ledgerState(s, data) === 'working') parts.push(a.activity);
  const subs = a.subagents?.length || 0;
  if (subs > 0) parts.push(`サブエージェント ${subs}`);
  if (a.pending) parts.push('ターンは終わり、サブエージェントの終了待ち');
  return parts.join(' · ');
}

/* waiting / permission / stopped / idle / done / failed / working / unknown / ended, or none for a worktree that has no session. The
   record says a worker is gone but not why, so a worker that is gone at a phase where its work
   is done reads as ended, as its card does. */
function sessionState(s, data = state) {
  if (data === state && restartingNow(s)) return 'restarting';
  if (s.waiting) return 'waiting';
  return restingState(s, data);
}

/* `sessionState` without the gate: whether the session is running, stopped or finished, which a
   session that waits on a gate is too. The server sets `waiting` for one that is not running. */
function restingState(s, data = state) {
  if (s.kind === 'hub') {
    if (s.present) return ledgerState(s, data);
    // A parent-task hub none of whose checkouts report to it any more is finished.
    const h = (data.hubs || []).find(x => x.id === s.id);
    return h && h.parent && !h.children ? 'ended' : 'stopped';
  }
  if (s.present) return ledgerState(s, data);
  if (s.stale) return FINISHED_PHASES.includes(s.phase) ? 'ended' : 'stopped';
  return s.conversation ? 'ended' : 'none';
}

/* What a row and the panel's head call a session: the hub's name, or the worktree's directory. */
function sessionKey(s, data = state) {
  if (s.kind === 'hub') {
    const h = (data.hubs || []).find(x => x.id === s.id);
    return h ? hubShortName(h) : s.id;
  }
  return baseName(s.worktree) || s.id;
}

/* What a row and the panel's head title a session with: `title` is the repository's name for its
   hub, the parent task's title for a parent-task hub and the task's title for a worker, empty
   while it is not known; `key` is what sits beside it (the hub's key, the worktree's name).
   `ask` lets a worker's task be asked of its own board, which the rows never do themselves. */
function sessionTitle(s, ask = false, data = state) {
  if (s.kind === 'hub') {
    const h = (data.hubs || []).find(x => x.id === s.id);
    if (!h) return { key: '', title: '' };
    return { key: h.parent ? h.key || '' : '', title: hubTitle(h, data) || '' };
  }
  return { key: sessionKey(s, data), title: s.taskTitle || boardTaskTitle(s, ask) };
}

/* The words of the row's main line: the title with its key beside it, or the key alone while
   there is no title. The key is left out when it would only repeat the text. */
function sessionLabel(s, ask = false, data = state) {
  const { key, title } = sessionTitle(s, ask, data);
  const text = title || sessionKey(s, data);
  return { tag: key && key !== text ? key : '', text };
}

function lastOutputText(s, data = state) {
  if (data.now == null || s.lastActivityAt == null) return null;
  return agoLabel(minutesSince(s.lastActivityAt, data.now));
}

/* `s` as the list and the address name it: a session of 「すべて」 is told apart by the board it
   was read from, since a worker's id is only its repository's. */
const sessionRef = s => s._slug ? `${s._slug}/${s.id}` : s.id;

/* ── What the page keeps of sessions ── */
const sessView = {
  starts: [],        // sessions this page asked a hub for and has not seen start (sessions-start.js)
  dismissed: [],     // pending rows closed on this page, by key
  boards: {},        // slug -> { data, at, key, loading, error }: other boards' state, read for titles
  git: null,         // { id, at, data, error, loading }: the panel's session's git state (sessions-side.js)
  screens: {},       // session id -> { lines, at }: the last lines this page saw of its terminal
};
const sessEl = id => document.getElementById(id);

/* The tmux pane is gone once the agent exits, so what the person last saw of it exists only in
   this page: kept, for the panel over a session that is not running. */
function keepScreen(rec) {
  const lines = rec.term?.snapshot?.() || [];
  if (lines.length) sessView.screens[rec.sessionId] = { lines, at: state.now };
}

/* What the task panel opens `session:<id>` as: a hub is the hub's, a worker whose task is on the
   board is that task (`taskOfSession`), and any other session stays itself. Other refs are returned as they are. */
function panelRefOf(id) {
  const s = sessOfRef(id);
  if (!s) return id;
  if (s.kind === 'hub') return HUB_REF + s.id;
  // The work view opens a worker on this board when the board of its task is not served: a bare task
  // id here would be the repository's own task of that id.
  if (view === 'work' && s.task && boardOfSession(s).slug && boardOfSession(s).slug !== nav.board) return id;
  const t = taskOfSession(s);
  return t ? t.id : id;
}

/* A session, in the panel: on a resident server in 「いまの仕事」, on the board its hub belongs to, on its task when it has one and
   as the hub or as itself otherwise; a board served alone has no list of work, and the panel opens over its board. In 「すべて」
   a session is another board's, told apart by the board it was read from. */
function openSessionRef(ref) {
  const s = (state.sessions || []).find(x => sessionRef(x) === ref);
  if (!s) return;
  const owned = taskOfSession(s);
  const subject = s.kind === 'hub' ? HUB_REF + s.id : owned ? owned.id : SESS_REF + s.id;
  if (!multiBoard) return openTaskPanel(subject, hasSession(s) ? 'term' : 'detail');
  go({ board: s._slug || boardOfSession(s).slug || nav.board, view: 'work', task: subject, pane: 'detail' }, { replace: subject === nav.task });
}

/* A worker's task title when the server did not give one. The task of another board is not in
   this page's state: asked of that board once (when `ask`), and remembered. */
function boardTaskTitle(s, ask) {
  if (!s.task) return '';
  // 「すべて」 holds every board's tasks, told apart by the board they came from.
  if (scopeAll()) return (state.tasks || []).find(t => t.id === s.task && t._slug === s._slug)?.title || '';
  const own = (state.tasks || []).find(t => t.id === s.task);
  if (own) return own.title || '';
  const b = boardOfSession(s);
  if (b.own || !b.slug) return '';
  const known = sessView.boards[b.slug];
  if (!known) {
    if (ask) loadSideBoard(b.slug, b.base);
    return '';
  }
  return (known.data?.tasks || []).find(t => t.id === s.task)?.title || '';
}

/* A row's tooltip: the whole title, where the worktree is, the tab's own title when it says
   something else, and how the session is. */
function sessionTip(s, st, data = state) {
  const { title } = sessionTitle(s, false, data);
  const where = s.kind === 'hub' ? '' : `${sessionKey(s, data)}${s.branch ? ` (${s.branch})` : ''}`;
  const sub = st ? [STATE_LABEL[st], s.present ? lastOutputText(s, data) : null].filter(Boolean).join(' · ') : '';
  // The ledger's own word, as it was written: only here and in the panel, and never as a class.
  const said = s.present && s.agentSession?.status ? `エージェントの報告: ${s.agentSession.status}` : '';
  const noRow = s.present && !s.agentSession ? unknownWhy(s) : '';
  return [title, where, s.title && s.title !== title && s.kind !== 'hub' ? s.title : '', sub, agentText(s, data), said, noRow].filter(Boolean).join('\n');
}

/* What a row says under its title: what it asks permission for, else the last line the session
   wrote, else what the agent's hooks say (which is all an iTerm2 row has), else when it last
   wrote. The sub-agents are counted whichever of them it was. */
function sessionLastText(s, data) {
  const asks = !s.waiting && agentStateOf(s) === 'permission' && s.agentSession.request;
  const when = s.present ? lastOutputText(s, data) : null;
  const n = s.present ? s.agentSession?.subagents?.length || 0 : 0;
  const counted = n > 0 ? `サブエージェント ${n}` : '';
  // `agentText` has the count in it already; the others do not.
  const add = text => counted ? `${text}${text ? ' · ' : ''}${counted}` : text;
  if (asks) return add(requestText(s));
  if (s.lastLine) return add(s.lastLine);
  return agentText(s, data) || add(when ? `最後の出力: ${when}` : '');
}

/* The element `html` makes. */
const htmlNode = html => {
  const t = document.createElement('template');
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
};
