/* The セッション tab of a board: its hubs, each with its sessions, in a list. A session opens in
   the task panel, as a task does (task-panel.js). The first half is pure (it reads `state` and
   returns); the second draws the list. What can be done to a session is in session-actions.js. */

/* A worker whose tmux window has been quiet this long reads as idle (seconds; window_activity
   is the only clock there is, so this is a guess at what "quiet" means). */
const IDLE_AFTER_SECS = 60;
/* The phases at which a worker that is gone has finished rather than stopped (see stuckOf). */
const FINISHED_PHASES = ['pr', 'pr-bots', 'review', 'report'];
const STATE_ORDER = { waiting: 0, stopped: 1, restarting: 1, idle: 2, working: 3, ended: 4, none: 5 };
const STATE_LABEL = {
  waiting: '確認待ち', stopped: '停止', idle: '待機中（出力なし）', working: '作業中', ended: '終了', none: 'セッションなし',
  pending: '起動を依頼中…', restarting: '再起動しています…',
};
const STATE_ICON = {
  waiting: 'help', stopped: 'error', idle: 'hourglass_empty', working: 'play_circle', ended: 'check_circle', none: 'remove_circle_outline', pending: 'hourglass_top', restarting: 'autorenew',
};
const STATE_PILL = {
  waiting: 'pill-warn', stopped: 'pill-err', idle: 'pill-neutral', working: 'pill-good', ended: 'pill-blue', none: 'pill-neutral', restarting: 'pill-neutral',
};
/* A row's state in the few words it has room for. A quiet window is running too: it only has
   the neutral pill (STATE_PILL), so it is not taken for one that is writing. */
const ROW_LABEL = { waiting: '入力待ち', stopped: '停止', idle: '稼働', working: '稼働', ended: '終了', restarting: '再起動' };

const hubOfSession = s => s.kind === 'hub' ? s.id : s.hub;

/* The one place that decides which board a session belongs to. A worker's `hub` names a
   `hubs[]` entry, and that entry's slug is its board: `/b/<slug>` on the resident server.
   Its API is read from there (`sessionApi`). The terminal socket is not: it stays on this
   page's board, which lists every linked worktree's session, while a parent-task hub's slug
   may be missing from the resident server's address book. */
function boardOfSession(s) {
  const hubId = hubOfSession(s);
  const hub = (state.hubs || []).find(h => h.id === hubId) || null;
  const base = state.resident && hub?.slug ? `/b/${hub.slug}` : BASE;
  return { hubId, hub, slug: hub?.slug || null, base, own: base === BASE };
}

/* `api`, but against the board the session belongs to. */
const sessionApi = (s, path, options) => boardApi(boardOfSession(s).base, path, options);

/* The functions below read `data`, a state: the page's own, or one repository's part of 「すべて」,
   where a hub's id is only its repository's. */
function sessionActivity(s, data = state) {
  return data.now != null && s.lastActivityAt != null && data.now - s.lastActivityAt >= IDLE_AFTER_SECS
    ? 'idle' : 'working';
}

/* waiting / stopped / idle / working / ended, or none for a worktree that has no session. The
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
    if (s.present) return sessionActivity(s, data);
    // A parent-task hub none of whose checkouts report to it any more is finished.
    const h = (data.hubs || []).find(x => x.id === s.id);
    return h && h.parent && !h.children ? 'ended' : 'stopped';
  }
  if (s.present) return sessionActivity(s, data);
  if (s.stale) return FINISHED_PHASES.includes(s.phase) ? 'ended' : 'stopped';
  return s.conversation ? 'ended' : 'none';
}

/* What a row and the panel's head call a session: the hub's name, or the worktree's directory. */
function sessionKey(s, data = state) {
  if (s.kind === 'hub') {
    const h = (data.hubs || []).find(x => x.id === s.id);
    return h ? hubShortName(h) : s.id;
  }
  return (s.worktree || '').split('/').filter(Boolean).pop() || s.id;
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
  const mins = Math.max(0, Math.floor((data.now - s.lastActivityAt) / 60));
  return mins < 1 ? 'たった今' : `${minutesLabel(mins)}前`;
}

/* `s` as the list and the address name it: a session of 「すべて」 is told apart by the board it
   was read from, since a worker's id is only its repository's. */
const sessionRef = s => s._slug ? `${s._slug}/${s.id}` : s.id;

/* The groups: each hub with its workers, sorted by state within a hub. Hubs stay in the order
   the server gives (the repository's first) — sorting them by state would move them under the
   pointer. Workers whose hub is not listed go in a group of their own at the end, and a
   worktree with no session sits with the hub it belongs to. */
function sessionTree(data = state, pendingHubs = new Set()) {
  const sessions = data.sessions || [];
  const groups = [];
  const byId = new Map();
  const group = (id, hub) => {
    let g = byId.get(id);
    if (!g) {
      g = { id, hub, hubSession: null, rows: [], orphans: [], unknown: !hub };
      byId.set(id, g);
      groups.push(g);
    }
    return g;
  };
  for (const h of data.hubs || []) group(h.id, h);
  const listed = groups.length;
  for (const s of sessions) {
    if (s.kind === 'hub') { group(s.id, null).hubSession = s; continue; }
    const st = sessionState(s, data);
    const g = group(hubOfSession(s) || 'hub', null);
    if (st === 'none') g.orphans.push(s); else g.rows.push({ s, state: st });
  }
  // Unlisted groups after the listed ones, by id.
  const tail = groups.splice(listed).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  groups.push(...tail);
  for (const g of groups) {
    if (g.hub && !g.hubSession) g.hubSession = sessions.find(s => s.kind === 'hub' && s.id === g.id) || null;
    // A hub with no session of its own is only as alive as its record says.
    g.own = g.hubSession || { kind: 'hub', id: g.id, present: !!g.hub?.state?.present };
    g.state = sessionState(g.own, data);
    g.rest = restingState(g.own, data);
    g.short = g.hub ? hubShortName(g.hub) : g.id === 'hub' ? 'リポジトリ' : g.id.replace(/^hub-/, '');
    g.title = g.hub ? hubTitle(g.hub, data) : g.id === 'hub' ? repoName(data) : null;
    g.text = g.title || g.short;
    // The repository's own hub says only the name; a parent task's key goes beside its title.
    g.tag = g.title && g.hub?.parent && g.hub.key ? g.hub.key : '';
    g.label = g.hub ? hubLabel(g.hub)
      : g.id === 'hub' ? 'リポジトリの hub' : `親タスク ${g.short} の hub（一覧にありません）`;
    g.child = g.hub ? !!g.hub.parent : g.id !== 'hub';
    g.rows.sort((a, b) =>
      STATE_ORDER[a.state] - STATE_ORDER[b.state]
      || (a.state === 'waiting' ? (stampSecs(a.s.waiting.openedAt) || 0) - (stampSecs(b.s.waiting.openedAt) || 0) : 0)
      || (a.s.id < b.s.id ? -1 : a.s.id > b.s.id ? 1 : 0));
    g.orphans.sort((a, b) => sessionKey(a, data) < sessionKey(b, data) ? -1 : sessionKey(a, data) > sessionKey(b, data) ? 1 : 0);
    // What the header counts: workers listed, and the sessions (a hub's own too) that wait.
    g.count = g.rows.length;
    g.waiting = g.rows.filter(r => r.state === 'waiting').length + (g.own.waiting ? 1 : 0);
    g.data = data;
  }
  // A parent task's hub that is over, with nothing left to show, is not worth a header.
  return groups.filter(g => !(g.hub?.parent && g.rest === 'ended'
    && !g.rows.length && !g.orphans.length && !pendingHubs.has(g.id)));
}

/* The groups the tab lists, in the order it shows them: a repository's board lists its hub and
   its parent tasks' hubs; a parent task's board only its own; 「すべて」 every repository's, in
   the sidebar's order. `pending` adds the rows for sessions asked for and not started yet. */
function sessionGroups({ pending = false } = {}) {
  const pend = pending && !scopeAll() ? sessionPendingRows() : [];
  const pendByHub = new Map();
  for (const p of pend) pendByHub.set(p.hubId, [...(pendByHub.get(p.hubId) || []), p]);
  const asked = new Set(pendByHub.keys());
  let groups = [];
  if (scopeAll()) {
    // A repository with boards of parent tasks only has several carriers, each listing its own
    // hub: they are one repository's part, each hub and session once.
    const repos = new Map();
    for (const c of state.carriers || []) repos.set(c.nwo, [...(repos.get(c.nwo) || []), c.slug]);
    const once = list => list.filter((x, i) => list.findIndex(y => y.id === x.id) === i);
    for (const [nwo, slugs] of repos) {
      const part = {
        repo: nwo,
        now: state.now,
        hubs: once((state.hubs || []).filter(h => slugs.includes(h._slug))),
        sessions: once((state.sessions || []).filter(x => slugs.includes(x._slug))),
      };
      for (const g of sessionTree(part)) groups.push({ ...g, slug: slugs[0], gid: `${slugs[0]}/${g.id}`, pending: [] });
    }
  } else {
    const own = pageHub();
    groups = sessionTree(state, asked)
      .filter(g => !own?.parent || g.id === own.id)
      .map(g => ({ ...g, slug: null, gid: g.id, pending: pendByHub.get(g.id) || [] }));
  }
  return groups;
}

/* What the tab says it lists. */
function sessionScopeText() {
  const own = pageHub();
  const what = scopeAll() ? 'すべての hub' : own?.parent ? 'この親タスクの hub だけ' : 'このリポジトリの hub と、配下の親タスク hub';
  return `${what}。hub ごとにまとめ、押すとターミナルが開きます`;
}

/* ── The tab ── */
function renderSessionsTab() {
  const tab = document.getElementById('tab-sessions');
  if (!tab) return;
  const available = !!state.boardTerminal?.available;
  tab.hidden = !available;
  if (!available) return;
  // 「すべて」 reads sessions only while this tab is shown: until a round has brought them, the
  // counts are not known, and a 0 would be wrong.
  const unknown = scopeAll() && !state.sessionsRead;
  if (unknown) {
    document.getElementById('sessions-waiting').classList.add('zero');
    document.getElementById('sessions-count').classList.add('zero');
    return;
  }
  // The sum of what the groups' headers say; sessions asked for and not started yet are not any.
  let waiting = 0;
  let count = 0;
  for (const g of sessionGroups()) { waiting += g.waiting; count += g.count; }
  const badge = document.getElementById('sessions-waiting');
  badge.textContent = waiting;
  badge.classList.toggle('zero', !waiting);
  const total = document.getElementById('sessions-count');
  total.textContent = count;
  total.classList.toggle('zero', !count);
}

/* ── The view ── */
const sessView = {
  pending: false,    // the address names this tab; opened once the first poll says whether a terminal exists
  listStructure: '', // what the list was last built from (renderSessionList)
  listRows: new Map(), // each row's own signature, by `row:<ref>` and `head:<group id>`
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

function leaveSessionsView() {
  if (/^#sessions?(\/|$)/.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
}

/* Draw the tab; what is open in the panel is the address's `task=`. */
function openSessionsView() {
  if (!state.boardTerminal?.available) return;
  if (view !== 'sessions') setView('sessions');
}

/* An address that arrived before the first poll had said whether this board has terminals. */
function openPendingSession() {
  // No terminal key yet means no poll has succeeded; the next one comes back here.
  if (!sessView.pending) return;
  if (nav.view !== 'sessions') { sessView.pending = false; return; }
  if (state.boardTerminal === undefined) return;
  sessView.pending = false;
  if (state.boardTerminal?.available) {
    openSessionsView();
    // The poll that told us had no `lines=1`: ask again now that this tab is on screen.
    if (view === 'sessions') refresh(true);
  } else giveUpSessions();
}

/* Where there are no terminals the tab is not there: the address and the screen go back to the
   board that was last shown. */
function giveUpSessions() {
  nav.view = prefs.tab === 'agent' ? 'agent' : 'human';
  if (isSessRef(nav.task)) { nav.task = null; nav.pane = 'detail'; }
  history.replaceState(null, '', urlOf());
  if (view !== 'board') setView('board'); else applyLayout();
  if (isSessRef(selectedTaskId)) hideTaskPanelState();
}

/* What the task panel opens `session:<id>` as: a hub is the hub's, a worker whose task is on the
   board is that task (`taskOfSession`), and any other session stays itself. Other refs are returned as they are. */
function panelRefOf(id) {
  const s = sessOfRef(id);
  if (!s) return id;
  if (s.kind === 'hub') return HUB_REF + s.id;
  const t = taskOfSession(s);
  return t ? t.id : id;
}

/* A row of the list. In 「すべて」 the session is another board's: that board is shown first. */
function openSessionRef(ref) {
  const s = (state.sessions || []).find(x => sessionRef(x) === ref);
  if (!s) return;
  // A worker of a task on the board is talked to in the task panel, over the list.
  const owned = taskOfSession(s);
  if (owned) {
    return go({ ...(scopeAll() ? { board: s._slug } : {}), view: 'sessions', task: owned.id, pane: 'term' },
      { replace: !scopeAll() && owned.id === nav.task });
  }
  // The hub's board, when this page knows it: where a worker's task is, whichever board this is.
  const hub = scopeAll() ? (state.hubs || []).find(h => h._slug === s._slug && h.id === hubOfSession(s)) : boardOfSession(s).hub;
  const known = hub?.slug && boards.some(b => b.slug === hub.slug) ? hub.slug : null;
  // A worker whose task is on the board of another hub: that board shows it as a task.
  if (!scopeAll() && s.kind === 'worker' && s.task && known && !boardOfSession(s).own) {
    return go({ board: known, view: 'sessions', task: s.task, pane: 'term' });
  }
  // A session with no task opens in the panel as itself, until it is linked to one.
  const task = SESS_REF + s.id;
  if (scopeAll()) return go({ board: known || s._slug, view: 'sessions', task, pane: 'term' });
  go({ view: 'sessions', task, pane: hasSession(s) ? 'term' : 'detail' }, { replace: task === nav.task });
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
  return [title, where, s.title && s.title !== title && s.kind !== 'hub' ? s.title : '', sub].filter(Boolean).join('\n');
}

/* What a row says under its title: the last line the session wrote, else when it last did. */
function sessionLastText(s, data) {
  if (s.lastLine) return s.lastLine;
  const when = s.present ? lastOutputText(s, data) : null;
  return when ? `最後の出力: ${when}` : '';
}

/* A worker's row, or a worktree's with no session (state `none`). */
function sessionRowHtml(s, st, g) {
  const data = g.data;
  const label = sessionLabel(s, false, data);
  const phase = st === 'working' && s.phase ? agentLabel(AGENT_COL_OF_PHASE[s.phase]) || s.phase : '';
  const pill = st === 'none' ? ''
    : `<span class="m3-pill sess-row-pill ${STATE_PILL[st]}">${esc(ROW_LABEL[st] + (phase ? ` · ${phase}` : ''))}</span>`;
  const last = st === 'none' ? s.branch || '' : sessionLastText(s, data);
  return `<button type="button" class="sess-row ${st}" data-sref="${esc(sessionRef(s))}" title="${esc(sessionTip(s, st, data))}">
    ${pill}<span class="sess-row-main"><span class="sess-row-title">${esc(label.text)}</span><span class="sess-row-last">${esc(last)}</span></span>
    <span class="sess-row-tag">${esc(label.tag)}</span><span class="sess-row-open">開く</span></button>`;
}

/* How a hub is, in the words the header has room for. */
function groupPill(g) {
  if (g.state === 'waiting') return ['hub が入力待ち', 'pill-warn'];
  if (g.state === 'working') return ['稼働中', 'pill-good'];
  if (g.state === 'idle') return ['稼働中', 'pill-neutral'];
  if (g.state === 'ended') return ['終了', 'pill-neutral'];
  const since = g.hub && multiBoard ? sinceLabel(boards.find(b => b.slug === g.hub.slug)?.hubLastAlive) : '';
  return [`停止中${since ? ` · ${since}` : ''}`, 'pill-err'];
}

/* The board a hub is on, for what the tab does to it from 「すべて」. */
const hubBoardOf = g => (g.hub?.slug && boards.find(b => b.slug === g.hub.slug)) || boards.find(b => b.slug === g.slug) || null;

/* The header's button: the hub's terminal, or the start of a hub that is stopped, with what keeps
   it from working as its tooltip. */
function groupButton(g) {
  if (g.rest === 'ended' || !g.hub) return null;
  if (g.rest === 'stopped') {
    const row = scopeAll() ? hubBoardOf(g) : null;
    const starting = scopeAll() ? !!row && rowStartingNow(row) : hubStartingNow(g.hub);
    const why = hubStartWhy(g.hub) || (starting ? 'hub を起動しています' : '');
    return { act: 'hub-start', icon: 'play_arrow', label: 'hub を起動', why, title: why || 'tmux の新しいウィンドウで adj hub を実行します' };
  }
  // A hub outside tmux has no terminal to open, but its panel still shows what it handles.
  const ready = !!g.hubSession && boardTerminalReady(g.hubSession);
  return { act: 'hub-open', icon: 'terminal', label: 'hub', why: '', title: ready ? 'hub のターミナルを開く' : 'hub を開く（tmux の外で動いているため、端末は開けません）' };
}

/* A hub's header: stuck to the top of the list while its sessions scroll under it. */
function groupHeadHtml(g) {
  const [pillText, pillCls] = groupPill(g);
  const btn = groupButton(g);
  const tip = [g.title, g.label].filter(Boolean).join('\n');
  const count = `<span class="sess-head-n">${g.count} セッション</span>${g.waiting ? `<span class="sess-head-sep"> · </span><b>入力待ち ${g.waiting}</b>` : ''}`;
  return `<header class="sess-head${g.waiting ? ' wait' : ''}${g.rest === 'stopped' || g.rest === 'ended' ? ' off' : ''}" title="${esc(tip)}">
    <span class="material-symbols-outlined sess-head-ico" aria-hidden="true">${g.child ? 'account_tree' : 'folder'}</span>
    <span class="sess-head-name">${g.tag ? `<span class="key">${esc(g.tag)}</span>` : ''}${esc(g.text)}</span>
    <span class="m3-pill ${pillCls}">${esc(pillText)}</span>
    <span class="sess-head-count">${count}</span>
    ${btn ? `<button type="button" class="btn-m3-tonal sess-head-btn" data-hub-act="${btn.act}"${btn.act === 'hub-open' ? ` data-hub-ref="${esc(HUB_REF + g.id)}"` : ''}${btn.why ? ' disabled' : ''} title="${esc(btn.title)}"><span class="material-symbols-outlined" aria-hidden="true">${btn.icon}</span><span class="lbl">${esc(btn.label)}</span></button>` : ''}
  </header>`;
}

function groupBodyHtml(g, opened) {
  const orphans = g.orphans.length
    ? `<details class="sess-orphans" data-orphans="${esc(g.gid)}"${opened ? ' open' : ''}><summary><span class="material-symbols-outlined" aria-hidden="true">folder_off</span><span>セッションのない worktree（${g.orphans.length}）</span></summary>${g.orphans.map(s => sessionRowHtml(s, 'none', g)).join('')}</details>` : '';
  const empty = g.rows.length || g.pending.length ? ''
    : `<div class="sess-empty">${g.rest === 'stopped' ? 'hub が止まっているので worker は動いていません' : g.rest === 'ended' ? 'この hub は役目を終えています' : '動いている worker はいません'}</div>`;
  return g.pending.map(pendingRowHtml).join('') + g.rows.map(r => sessionRowHtml(r.s, r.state, g)).join('') + empty + orphans;
}

const groupHtml = (g, opened) =>
  `<section class="sess-group${g.child ? ' child' : ''}" data-gid="${esc(g.gid)}">${groupHeadHtml(g)}<div class="sess-body">${groupBodyHtml(g, opened)}</div></section>`;

/* The list. It is rebuilt only when what it is made of changes (a hub or a session comes or
   goes); otherwise only the rows whose own words changed are redrawn, so a poll that moves one
   line does not take the scroll position or the keyboard focus from the rest. */
function renderSessionList() {
  const groups = sessionGroups({ pending: true });
  sessEl('sess-scope').textContent = sessionScopeText();
  sessEl('sess-add').disabled = scopeAll();
  sessEl('sess-add').title = scopeAll() ? '追加するボードを選んでください（「すべて」からは追加できません）' : 'hub やセッションを追加';
  // 「すべて」 has its hubs before it has their sessions: drawn now, every hub would read as empty.
  if (scopeAll() && !state.sessionsRead) {
    // A round that asked and got no answer is not one still on its way; the next round retries.
    const token = state.sessionsAsked ? 'failed' : 'loading';
    if (sessView.listStructure !== token) {
      sessView.listStructure = token;
      sessView.listRows = new Map();
      sessEl('sess-groups').innerHTML = `<div class="sess-empty">${state.sessionsAsked ? 'セッションを読めませんでした' : '読み込み中…'}</div>`;
    }
    return;
  }
  const open = new Set(prefs.sessionsFolded || []);
  const opened = g => open.has(`orphans:${g.gid}`);
  // What each row draws, one signature per row, so that a row is redrawn only when its own
  // words change and not whenever any other row's do.
  const rowSig = (s, st, g) => JSON.stringify([sessionRef(s), st, sessionLabel(s, false, g.data), sessionTip(s, st, g.data), sessionLastText(s, g.data), s.phase || '']);
  const rows = new Map();
  for (const g of groups) {
    rows.set(`head:${g.gid}`, groupHeadHtml(g));
    for (const r of g.rows) rows.set(`row:${sessionRef(r.s)}`, rowSig(r.s, r.state, g));
    for (const s of g.orphans) rows.set(`row:${sessionRef(s)}`, rowSig(s, 'none', g));
  }
  // What the list is made of: a change here is rebuilt. The order of a group's rows is left
  // out, since moving the rows it already has is enough for that.
  const structure = JSON.stringify([
    groups.map(g => [g.gid, g.child, g.rest, g.rows.map(r => sessionRef(r.s)).sort(), g.orphans.map(sessionRef), opened(g),
      // Part of the signature: without it a row that appears or changes its words while nothing
      // else does would stay hidden behind the redraw skip.
      g.pending.map(p => [p.key, p.hubId, p.name, p.kind, p.text, p.canStart, p.busy])]),
  ]);
  const root = sessEl('sess-groups');
  if (structure === sessView.listStructure) {
    patchSessionList(root, groups, rows);
    return;
  }
  sessView.listStructure = structure;
  sessView.listRows = rows;
  const scroll = document.querySelector('.sess-list-scroll');
  const top = scroll.scrollTop;
  const focused = document.activeElement?.closest?.('#sess-groups') ? document.activeElement : null;
  // What had the focus, found again in the new list: a row, a header's button or a fold.
  const gidOf = el => el?.closest('.sess-group')?.dataset.gid;
  const held = !focused ? null : focused.dataset.sref != null ? { ref: focused.dataset.sref }
    : focused.dataset.hubAct ? { gid: gidOf(focused), act: focused.dataset.hubAct }
      : focused.dataset.pendAct ? { pend: focused.dataset.pend, pendAct: focused.dataset.pendAct }
        : focused.matches('.sess-orphans > summary') ? { gid: gidOf(focused), fold: true } : null;
  root.innerHTML = groups.map(g => groupHtml(g, opened(g))).join('') || '<div class="sess-empty">表示できる hub がありません</div>';
  scroll.scrollTop = top;
  if (held) {
    const group = held.gid != null ? [...root.querySelectorAll('.sess-group')].find(el => el.dataset.gid === held.gid) : null;
    const again = held.pend ? [...root.querySelectorAll('[data-pend-act]')].find(b => b.dataset.pend === held.pend && b.dataset.pendAct === held.pendAct)
      : held.ref != null ? [...root.querySelectorAll('.sess-row[data-sref]')].find(row => row.dataset.sref === held.ref)
      : held.act ? group?.querySelector(`[data-hub-act="${held.act}"]`) : group?.querySelector('.sess-orphans > summary');
    again?.focus({ preventScroll: true });
  }
}

/* `old` replaced by the node `html` makes, keeping what a redraw must not take from a row:
   the selection and the keyboard focus. */
const htmlNode = html => {
  const t = document.createElement('template');
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
};
function swapSessionRow(old, html) {
  const node = htmlNode(html);
  const focused = document.activeElement === old;
  if (old.hasAttribute('aria-current')) node.setAttribute('aria-current', 'true');
  old.replaceWith(node);
  if (focused) node.focus({ preventScroll: true });
  return node;
}

/* The list as it stands, brought up to `groups` without rebuilding it: only the rows whose own
   signature changed are redrawn, and the rows of a group that came in another order are
   moved. What is on screen keeps its scroll position, its focus and its selection. */
function patchSessionList(root, groups, rows) {
  const before = sessView.listRows;
  const changed = key => before.get(key) !== rows.get(key);
  // Moving a node blurs it, so the focused row is found again afterwards.
  const focused = document.activeElement;
  const focusedRef = focused?.closest?.('#sess-groups') ? focused.dataset?.sref : null;
  groups.forEach((g, i) => {
    const el = root.children[i];
    if (changed(`head:${g.gid}`)) {
      const head = el.querySelector(':scope > .sess-head');
      const node = htmlNode(groupHeadHtml(g));
      const had = head.contains(document.activeElement) ? document.activeElement.dataset.hubAct : null;
      head.replaceWith(node);
      if (had) node.querySelector(`[data-hub-act="${had}"]`)?.focus({ preventScroll: true });
    }
    const box = el.querySelector(':scope > .sess-body');
    const byRef = new Map();
    for (const row of box.querySelectorAll('.sess-row[data-sref]')) byRef.set(row.dataset.sref, row);
    let next = box.querySelector(':scope > .sess-row[data-sref]');
    for (const r of g.rows) {
      const old = byRef.get(sessionRef(r.s));
      let node = old;
      if (changed(`row:${sessionRef(r.s)}`)) {
        node = swapSessionRow(old, sessionRowHtml(r.s, r.state, g));
        if (old === next) next = node;
      }
      if (node === next) next = next.nextElementSibling;
      else box.insertBefore(node, next);
    }
    // Worktrees with no session keep their order: a change of it is a change of structure.
    for (const s of g.orphans) {
      if (changed(`row:${sessionRef(s)}`)) swapSessionRow(byRef.get(sessionRef(s)), sessionRowHtml(s, 'none', g));
    }
  });
  sessView.listRows = rows;
  if (focusedRef != null && document.activeElement !== focused) {
    const again = focused.isConnected ? focused
      : [...root.querySelectorAll('.sess-row[data-sref]')].find(row => row.dataset.sref === focusedRef);
    again?.focus({ preventScroll: true });
  }
}

function applySessionSelection() {
  // The row of the session the task panel is open on is the one in use: a task's worker, or
  // the session itself while it has no task.
  const task = selectedTaskId && !isHubRef(selectedTaskId) && !isSessRef(selectedTaskId) && !scopeAll() ? taskById(selectedTaskId) : null;
  const inPanel = isSessRef(selectedTaskId) && !scopeAll() ? sessIdOfRef(selectedTaskId) : task ? sessionOfTask(task)?.id : null;
  markHubButtons();
  for (const row of document.querySelectorAll('#sess-groups .sess-row[data-sref]')) {
    if (inPanel && row.dataset.sref === inPanel) row.setAttribute('aria-current', 'true');
    else row.removeAttribute('aria-current');
  }
}

function renderSessionsView() {
  if (view !== 'sessions') return;
  // A round of 「すべて」 that has no lead board yet says nothing either way: the tab stays.
  if (!state.boardTerminal?.available) {
    if (state.boardTerminal) giveUpSessions();
    return;
  }
  renderSessionList();
  applySessionSelection();
}
