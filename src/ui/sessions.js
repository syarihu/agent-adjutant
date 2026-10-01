/* The セッション tab of a board: its hubs, each with its sessions, in a list; the selected
   one's terminal fills the rest beside it. The first half is pure (it reads `state` and
   returns); the second draws and drives the terminal. */

/* A worker whose tmux window has been quiet this long reads as idle (seconds; window_activity
   is the only clock there is, so this is a guess at what "quiet" means). */
const IDLE_AFTER_SECS = 60;
/* The phases at which a worker that is gone has finished rather than stopped (see stuckOf). */
const FINISHED_PHASES = ['pr', 'pr-bots', 'review', 'report'];
const STATE_ORDER = { waiting: 0, stopped: 1, idle: 2, working: 3, ended: 4, none: 5 };
const STATE_LABEL = {
  waiting: '確認待ち', stopped: '停止', idle: '待機中（出力なし）', working: '作業中', ended: '終了', none: 'セッションなし',
  pending: '起動を依頼中…',
};
const STATE_ICON = {
  waiting: 'help', stopped: 'error', idle: 'hourglass_empty', working: 'play_circle', ended: 'check_circle', none: 'remove_circle_outline', pending: 'hourglass_top',
};
const STATE_PILL = {
  waiting: 'pill-warn', stopped: 'pill-err', idle: 'pill-neutral', working: 'pill-good', ended: 'pill-blue', none: 'pill-neutral',
};
const BACK_LABEL = { board: 'ボード', review: '要対応レビュー', task: 'タスク詳細', sessions: 'セッション' };
/* A row's state in the few words it has room for. A quiet window is running too: it only has
   the neutral pill (STATE_PILL), so it is not taken for one that is writing. */
const ROW_LABEL = { waiting: '入力待ち', stopped: '停止', idle: '稼働', working: '稼働', ended: '終了' };

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

/* What a row and the context bar call a session: the hub's name, or the worktree's directory. */
function sessionKey(s, data = state) {
  if (s.kind === 'hub') {
    const h = (data.hubs || []).find(x => x.id === s.id);
    return h ? hubShortName(h) : s.id;
  }
  return (s.worktree || '').split('/').filter(Boolean).pop() || s.id;
}

/* What a row and the context bar title a session with: `title` is the repository's name for its
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
  selectedId: null,
  last: null,        // the selected session as last listed, kept after it drops off the list
  back: null,        // { view, taskId } to return to, until the first switch to another session
  mounted: null,     // { sessionId, term, ended }
  pending: false,    // the address names this tab; opened once the first poll says whether a terminal exists
  listStructure: '', // what the list was last built from (renderSessionList)
  listRows: new Map(), // each row's own signature, by `row:<ref>` and `head:<group id>`
  starts: [],        // sessions this page asked a hub for and has not seen start (sessions-start.js)
  dismissed: [],     // pending rows closed on this page, by key
  boards: {},        // slug -> { data, at, key, loading, error }: other boards' state, read for titles and the sidebar
  sideNarrowOpen: false, // the sidebar below 1400px: floats over the terminal, never saved
  sideSig: '',
  sideHold: null,    // { id, released, timer }: the selected session's sidebar waits for its terminal (sessions-side.js)
  git: null,         // { id, at, data, error, loading }: the selected worktree's git state, for the sidebar
  screens: {},       // session id -> { lines, at }: the last lines this page saw of its terminal
  reconnectWhenReady: null, // a session just resumed or started: connect once its window exists
  gateError: null,   // { id, text } shown in the gate banner
  gateDraft: null,   // { id, text }: a comment typed in the banner, kept across redraws
  actionsSig: '',
  gateSig: '',
  overSig: '',
  overShown: null,
};
const sessEl = id => document.getElementById(id);
const currentSession = () => (state.sessions || []).find(s => s.id === sessView.selectedId) || null;

function detachSessionTerminal() {
  const m = sessView.mounted;
  if (!m) return;
  sessView.mounted = null;
  keepScreen(m);
  // Closing the socket only detaches; the session keeps running.
  m.term.dispose();
}

/* The tmux pane is gone once the agent exits, so what the person last saw of it exists only in
   this page: kept, for the panel over a session that is not running. */
function keepScreen(rec) {
  const lines = rec.term?.snapshot?.() || [];
  if (lines.length) sessView.screens[rec.sessionId] = { lines, at: state.now };
}

function leaveSessionsView() {
  detachSessionTerminal();
  document.body.classList.remove('sess-term-open');
  if (/^#sessions?(\/|$)/.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
}

/* Draw the tab with `id` open (or none): what the address `session=` names. */
function openSessionsView(id = null) {
  if (!state.boardTerminal?.available) return;
  if (view !== 'sessions') {
    // Below 1400px the sidebar floats over the terminal, so it starts out of the way.
    sessView.sideNarrowOpen = false;
    setView('sessions');
  }
  showSession(id);
  renderSessionContext();
}

/* An address that arrived before the first poll had said whether this board has terminals. */
function openPendingSession() {
  // No terminal key yet means no poll has succeeded; the next one comes back here.
  if (!sessView.pending) return;
  if (nav.view !== 'sessions') { sessView.pending = false; return; }
  if (state.boardTerminal === undefined) return;
  sessView.pending = false;
  if (state.boardTerminal?.available) {
    openSessionsView(nav.session);
    // The poll that told us had no `lines=1`: ask again now that this tab is on screen.
    if (view === 'sessions') refresh(true);
  } else giveUpSessions();
}

/* Where there are no terminals the tab is not there: the address and the screen go back to the
   board that was last shown. */
function giveUpSessions() {
  nav.view = prefs.tab === 'agent' ? 'agent' : 'human';
  nav.session = null;
  history.replaceState(null, '', urlOf());
  if (view !== 'board') setView('board'); else applyLayout();
}

/* Open the session `id` of this board beside the list: a step in the history. */
function selectSession(id) {
  if (!navApplying && (nav.view !== 'sessions' || nav.session !== id)) go({ view: 'sessions', session: id });
  else showSession(id);
}

/* A row of the list. In 「すべて」 the session is another board's: that board is shown first. */
function openSessionRef(ref) {
  const s = (state.sessions || []).find(x => sessionRef(x) === ref);
  if (!s) return;
  if (!scopeAll()) return selectSession(s.id);
  const hub = (state.hubs || []).find(h => h._slug === s._slug && h.id === hubOfSession(s));
  const slug = hub?.slug && boards.some(b => b.slug === hub.slug) ? hub.slug : s._slug;
  go({ board: slug, view: 'sessions', session: s.id });
}

function showSession(id) {
  const changed = sessView.selectedId !== id;
  if (changed) {
    sessView.back = null;
    sessView.last = null;
    sessView.git = null;
    sessView.reconnectWhenReady = null;
    sessView.gateError = null;
    showSessNotice('');
    detachSessionTerminal();
  } else if (id && sessView.mounted?.ended != null) {
    // The same row again is the way to connect once more.
    detachSessionTerminal();
  }
  sessView.selectedId = id;
  renderSessionsView();
}

function connectSelected() {
  detachSessionTerminal();
  renderSessionsView();
}

function returnFromSessions() {
  const back = sessView.back;
  sessView.back = null;
  // Opened from the list: back to the list.
  if (!back) return go({ session: null });
  // Through `go`, so that the address leaves the session as the screen does.
  if (back.view === 'review') return go({ view: 'review', session: null });
  go({ view: prefs.tab === 'agent' ? 'agent' : 'human', session: null });
  if (back.view === 'task' && back.taskId) return openTask(back.taskId);
  if (back.taskId && (back.view || 'board') === 'board' && (state.tasks || []).some(t => t.id === back.taskId)) selectTask(back.taskId);
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
    // While the selected session's terminal goes first, a sibling row of its hub waits too:
    // its board is the one the sidebar is holding back.
    const sel = currentSession();
    const held = sideHeld(sessView.selectedId) && (s.id === sel?.id || (sel && hubOfSession(sel) === hubOfSession(s)));
    if (ask && !held) loadSideBoard(b.slug, b.base, sideSignal(s));
    return '';
  }
  return (known.data?.tasks || []).find(t => t.id === s.task)?.title || '';
}

function renderSessionContext() {
  const id = sessView.selectedId;
  const s = currentSession() || (id && sessView.last?.id === id ? sessView.last : null);
  const gone = !!id && !currentSession() && sessView.last?.id === id;
  sessEl('sess-context').hidden = !id;
  sessEl('sess-back').hidden = !id;
  sessEl('sess-back').textContent = `← ${BACK_LABEL[sessView.back ? sessView.back.view : 'sessions'] || 'ボード'}に戻る`;
  if (!id) return;
  const label = s ? sessionLabel(s, true) : { tag: id, text: '' };
  const keyEl = sessEl('sess-key');
  keyEl.textContent = label.tag;
  keyEl.hidden = !label.tag;
  keyEl.title = s && s.kind !== 'hub' && s.branch ? `ブランチ: ${s.branch}` : '';
  const titleEl = sessEl('sess-title');
  titleEl.textContent = label.text;
  titleEl.title = s ? sessionTip(s) : '';
  const st = s && !gone ? sessionState(s) : null;
  const pill = sessEl('sess-state');
  pill.textContent = gone ? '一覧から消えました' : st ? STATE_LABEL[st] : 'セッションが見つかりません';
  pill.className = 'm3-pill ' + (st ? STATE_PILL[st] : 'pill-neutral');
  const hubEl = sessEl('sess-hub');
  const b = s && s.kind !== 'hub' ? boardOfSession(s) : null;
  hubEl.textContent = b ? (b.hub ? hubLabel(b.hub) : `${b.hubId}（一覧にありません）`) : '';
  hubEl.hidden = !b;
  const last = s && !gone ? lastOutputText(s) : null;
  sessEl('sess-last').textContent = `最後の出力: ${last || '不明'}`;
  const m = sessView.mounted;
  sessEl('sess-reconnect').hidden = !(m && m.ended != null && s && !gone && boardTerminalReady(s));
  const t = s?.terminal;
  sessEl('sess-strip-where').textContent = t?.backend === 'tmux' ? `tmux ${t.session || ''}:${t.window || ''}` : '';
  sessEl('sess-strip-attach').textContent = s?.attached == null ? ''
    : s.attached > 0 ? `ほかに ${s.attached} 個の端末がアタッチしています` : 'ほかの端末はアタッチしていません';
}

/* Why the selected session has no terminal, or null when it has one. */
function sessionPlaceholder(s) {
  const id = sessView.selectedId;
  if (!id) return 'セッションを選んでください';
  if (!s) return sessView.last ? 'このセッションは一覧から消えました' : 'セッションが見つかりません';
  const st = restingState(s);
  if (st === 'none') return `セッションはありません。\n${s.worktree}${s.branch ? `（${s.branch}）` : ''}`;
  if (st === 'stopped') return 'セッションは止まっています';
  if (st === 'ended') return 'セッションは終了しています';
  if (!s.present) return 'このセッションは動いていません';
  return 'このセッションの端末はボードから開けません（tmux で動いているセッションだけ開けます）';
}

function mountSelected() {
  const id = sessView.selectedId;
  const s = currentSession();
  // A terminal that ended, or whose session went away, stays on screen with its last lines;
  // it is replaced only by choosing a row, never by the poll.
  if (id && !sessView.mounted && s && boardTerminalReady(s)) {
    const rec = { sessionId: id, term: null, ended: null };
    sessView.mounted = rec;
    rec.term = mountSessionTerminal(sessEl('sess-term-host'), {
      sessionId: id,
      onReady: () => releaseSide(id),
      onEnd: code => {
        // A terminal that never got a frame has nothing more to wait for either.
        releaseSide(id);
        rec.ended = code;
        keepScreen(rec);
        if (sessView.mounted === rec) renderSessionsView();
      },
    });
  }
  const shown = !!sessView.mounted;
  sessEl('sess-term-host').hidden = !shown;
  const ph = sessEl('sess-placeholder');
  ph.hidden = shown;
  // A session that is not running says so in the panel, which carries what to do about it.
  const over = overHtml(s);
  const html = shown ? over : `<div class="sess-ph-body"><div>${esc(over ? '' : sessionPlaceholder(s))}</div>${over ? `<div class="sess-over">${over}</div>` : ''}</div>`;
  const overEl = sessEl('sess-over');
  overEl.hidden = !shown || !over;
  const target = shown ? overEl : ph;
  if (sessView.overSig !== html || sessView.overShown !== shown) {
    sessView.overSig = html;
    sessView.overShown = shown;
    (shown ? ph : overEl).innerHTML = '';
    target.innerHTML = html;
  }
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
  const ready = !!g.hubSession && boardTerminalReady(g.hubSession);
  return { act: 'hub-open', icon: 'terminal', label: 'hub', why: ready ? '' : 'tmux の外で動いている hub は、ボードから端末を開けません', title: ready ? 'hub のターミナルを開く' : 'tmux の外で動いている hub は、ボードから端末を開けません' };
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
    ${btn ? `<button type="button" class="btn-m3-tonal sess-head-btn" data-hub-act="${btn.act}"${btn.why ? ' disabled' : ''} title="${esc(btn.title)}"><span class="material-symbols-outlined" aria-hidden="true">${btn.icon}</span><span class="lbl">${esc(btn.label)}</span></button>` : ''}
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
    if (sessView.listStructure !== 'loading') {
      sessView.listStructure = 'loading';
      sessView.listRows = new Map();
      sessEl('sess-groups').innerHTML = '<div class="sess-empty">読み込み中…</div>';
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
  for (const row of document.querySelectorAll('#sess-groups .sess-row[data-sref]')) {
    if (!scopeAll() && row.dataset.sref === sessView.selectedId) row.setAttribute('aria-current', 'true');
    else row.removeAttribute('aria-current');
  }
}

function renderSessionsView() {
  if (view !== 'sessions') return;
  if (!state.boardTerminal?.available) { giveUpSessions(); return; }
  const cur = currentSession();
  if (cur) sessView.last = cur;

  // Resumed or started from here: the window comes a moment after the request returns.
  if (sessView.reconnectWhenReady && sessView.reconnectWhenReady === sessView.selectedId && boardTerminalReady(cur)) {
    sessView.reconnectWhenReady = null;
    // Detached first, which keeps the old run's screen; that screen must not be shown after a
    // later stop, so it is dropped before the new run connects.
    detachSessionTerminal();
    delete sessView.screens[cur.id];
    return renderSessionsView();
  }
  // With a session open the list narrows to a column beside it.
  const open = !!sessView.selectedId;
  sessEl('sessions-view').classList.toggle('term-open', open);
  document.body.classList.toggle('sess-term-open', open);
  renderSessionList();
  applySessionSelection();
  mountSelected();
  holdSideForSelection(cur);
  renderSessionContext();
  renderSessionActions(cur);
  renderSessionGate(cur);
  renderSessionSidebar();
}

/* ── Actions on the selected session, and the gate it waits on ── */
const sessBusy = new Set(); // in-flight requests: a second press is ignored and buttons dim
const enc = encodeURIComponent;

/* One row in the flow of the page, between the context bar and the terminal: never laid over
   the terminal, which refits to the height that is left. */
function showSessNotice(text, isError = false) {
  const el = sessEl('sess-notice');
  el.hidden = !text;
  el.className = 'sess-notice' + (isError ? ' err' : '');
  el.setAttribute('role', isError ? 'alert' : 'status');
  el.innerHTML = text
    ? `<span class="sess-notice-text">${esc(text)}</span><button type="button" class="btn-m3-text" data-sess-dismiss>閉じる</button>` : '';
}

async function sessAct(key, label, fn) {
  if (sessBusy.has(key)) return;
  sessBusy.add(key);
  renderSessionActions(currentSession());
  try {
    await fn();
  } catch (e) {
    note(`${label} → ${e.message}`, true);
    showSessNotice(`${label}: ${e.message}`, true);
  } finally {
    sessBusy.delete(key);
    renderSessionActions(currentSession());
  }
}

/* Why the board cannot start `h` (and so cannot reset it), or '' when it can. */
const hubStartWhy = h => h.parent && !h.key ? 'キーが分からないため起動できません。adj hub --hub <キー> で起動してください'
  : !state.hubStart?.available ? 'ボードからの起動は terminal.preset が "tmux" のときだけ使えます' : '';

/* What the board can do about a hub, by the same rule as its row in the hub list: a parent-task
   hub that no checkout reports to any more is closed, whether or not it runs. */
function hubActionOf(s) {
  const h = (state.hubs || []).find(x => x.id === s.id);
  if (!h) return null;
  const present = !!(s.present ?? h.state?.present);
  // A reset started from the rail leaves these two doing nothing until it is over.
  const resetting = hubResetting.has(h.id);
  const starting = hubStartingNow(h);
  if (h.parent && !h.children) return { act: 'hub-close', icon: 'close', label: 'hub を閉じる', disabled: starting, title: resetting ? 'hub をリセットしています' : starting ? 'hub を起動しています' : 'この hub を止めて一覧から外します。タスク・gate・受信箱の記録は残ります' };
  if (present) return { act: 'hub-stop', icon: 'stop', label: 'hub を止める', disabled: resetting, title: resetting ? 'hub をリセットしています' : 'hub が動いている tmux のペインを閉じます' };
  const why = hubStartWhy(h) || (hubStartingNow(h) ? 'hub を起動しています' : '');
  return { act: 'hub-start', icon: 'play_arrow', label: 'hub を起動', title: why || 'tmux の新しいウィンドウで adj hub を実行します', disabled: !!why };
}

const canResume = s => s.kind === 'worker' && !s.present && !!s.conversation && !!state.sessionResume?.available;
const linkedWorktree = s => s.kind === 'worker' && !!s.worktree && s.worktree !== state.main;

function sessionButtons(s) {
  const bar = [];
  const menu = [];
  if (canResume(s)) bar.push({ act: 'resume', icon: 'restart_alt', label: '再開', title: '保存された会話を tmux の新しいウィンドウで再開します' });
  if (s.kind === 'worker' && s.present && linkedWorktree(s)) bar.push({ act: 'close', icon: 'tab_close', label: 'セッションを閉じる', title: 'worker のタブを閉じます（worktree は残ります）' });
  if (s.kind === 'hub') {
    const hub = hubActionOf(s);
    if (hub) bar.push(hub);
    const h = (state.hubs || []).find(x => x.id === s.id);
    if (h) {
      const why = hubStartWhy(h) || (hubStartingNow(h) ? 'hub を起動しています' : '');
      menu.push({ act: 'hub-reset', icon: 'fiber_new', label: 'hub をリセット…', title: why || 'hub をリセット：新しい会話で hub を起動し直します（adj hub --new）', disabled: !!why });
    }
  }
  if (boardTerminalReady(s)) {
    const open = state.sessionOpen;
    bar.push(open?.available
      ? { act: 'open', icon: 'open_in_new', label: '端末で開く', title: `${open.terminal} で開く` }
      : { act: 'open', icon: 'open_in_new', label: '端末で開く', title: 'この端末からは開けません（terminal.attach が未設定で、iTerm2 も見つかりません）', disabled: true });
  }
  if (linkedWorktree(s)) menu.push({ act: 'ide', icon: 'code', label: 'IDE で開く' });
  if (s.worktree) menu.push({ act: 'copy', icon: 'content_copy', label: 'パスをコピー' });
  if (linkedWorktree(s)) menu.push({ act: 'cleanup', icon: 'delete_sweep', label: '片付ける…' });
  return { bar, menu };
}

const actionButtonHtml = (b, { menu = false, busy = false } = {}) =>
  `<button type="button"${menu ? ' role="menuitem"' : ` class="btn-m3-tonal sess-act"`} data-sess-act="${b.act}"${b.title ? ` title="${esc(b.title)}"` : ''}${b.disabled || busy ? ' disabled' : ''}>` +
  `<span class="material-symbols-outlined" aria-hidden="true">${b.icon}</span><span>${esc(b.label)}</span></button>`;

function renderSessionActions(s) {
  const busy = sessBusy.size > 0;
  const { bar, menu } = s ? sessionButtons(s) : { bar: [], menu: [] };
  const sig = JSON.stringify([s?.id, bar, menu, busy]);
  if (sig === sessView.actionsSig) return;
  sessView.actionsSig = sig;
  sessEl('sess-actions').innerHTML = bar.map(b => actionButtonHtml(b, { busy })).join('');
  sessEl('sess-menu').innerHTML = menu.map(b => actionButtonHtml(b, { menu: true, busy })).join('');
  sessEl('sess-menu-btn').hidden = !menu.length;
  if (!menu.length) closeSessMenu();
}

function closeSessMenu() {
  sessEl('sess-menu').hidden = true;
  sessEl('sess-menu-btn').setAttribute('aria-expanded', 'false');
}

function runSessionAction(act, s) {
  const key = sessionKey(s);
  const id = enc(s.id);
  switch (act) {
    case 'open':
      return sessAct('open', `端末で開く (${key})`, async () => {
        const data = await api(`/api/sessions/${id}/open`, { method: 'POST', body: '{}' });
        note(`端末で開く (${key})`, false, data.description);
        showSessNotice(data.description || '端末で開きました');
      });
    case 'close':
      return worktreeAct('close', s.worktree);
    case 'resume':
      return sessAct('resume', `セッションを再開 (${key})`, async () => {
        const data = await api(`/api/sessions/${id}/resume`, { method: 'POST', body: '{}' });
        note(`セッションを再開 (${key})`, false, data.description);
        showSessNotice('再開しました' + (data.hubRunning === false ? '。hub は止まっています' : ''));
        sessView.reconnectWhenReady = s.id;
        await refresh();
      });
    case 'hub-start':
      if ((state.hubs || []).some(x => x.id === s.id && hubStartingNow(x))) return showSessNotice('hub を起動しています');
      return sessAct('hub-start', `hub を起動 (${key})`, async () => {
        const data = await api(`/api/hubs/${id}/start`, { method: 'POST', body: '{}' });
        const text = data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました';
        note(`hub を起動 (${key})`, false, text);
        showSessNotice(text);
        sessView.reconnectWhenReady = s.id;
        await refresh();
      });
    case 'hub-reset':
      return openHubStopDialog(s.id, 'reset');
    case 'hub-stop':
    case 'hub-close':
      return openHubStopDialog(s.id, act === 'hub-stop' ? 'stop' : 'close');
    case 'ide':
      return worktreeAct('ide', s.worktree);
    case 'copy':
      // Without a secure context there is no clipboard object, and the call throws.
      return Promise.resolve().then(() => navigator.clipboard.writeText(s.worktree))
        .then(() => showSessNotice('パスをコピーしました'))
        .catch(e => {
          note(`パスをコピー (${sessionKey(s)}) → ${e.message}`, true);
          showSessNotice(`パスをコピーできませんでした: ${e.message}`, true);
        });
    case 'cleanup':
      return openCleanupDialog(s);
  }
}

/* ── The gate a session waits on ── */
const INLINE_GATES = ['plan', 'question', 'result', 'dispatch'];
const NEEDS_COMMENT = ['changes', 'ask', 'answer'];
const GATE_KIND_LABEL = { result: '結果の確認', issue: '起票の確認', relay: '連絡' };
const gateKindLabel = kind => GATE_KIND_LABEL[kind] || humanLabel(kind);
const DECISIONS = {
  approve: ['承認', 'check', 'primary'],
  changes: ['修正を指示', 'replay', 'tonal'],
  reject: ['却下', 'cancel', 'text'],
  ack: ['了解（閉じる）', 'check', 'primary'],
  ask: ['追加で聞く', 'help', 'tonal'],
  answer: ['これで返す', 'send', 'primary'],
};
/* A dispatch gate asks whether to start the task; the board's card says so in these words. */
const DISPATCH_DECISIONS = { approve: ['着手する', 'play_arrow', 'primary'], reject: ['Backlog に戻す', 'undo', 'tonal'] };

function gateButtonsHtml(w, busy) {
  const btn = (attr, [label, icon, cls]) =>
    `<button type="button" class="btn-m3-${cls} sess-act" ${attr}${busy ? ' disabled' : ''}><span class="material-symbols-outlined" aria-hidden="true">${icon}</span><span>${esc(label)}</span></button>`;
  if (w.choices?.length) {
    // The choices, then what else the gate offers besides approving one of them.
    const others = (w.options || []).filter(o => o !== 'approve' && (w.kind === 'dispatch' ? DISPATCH_DECISIONS : DECISIONS)[o]);
    return w.choices.map(c => btn(`data-sess-gate="choice" data-choice="${esc(c.id)}"`, [c.label, 'check', 'primary'])).join('') +
      others.map(o => btn(`data-sess-gate="${esc(o)}"`, (w.kind === 'dispatch' ? DISPATCH_DECISIONS : DECISIONS)[o])).join('');
  }
  const table = w.kind === 'dispatch' ? DISPATCH_DECISIONS : DECISIONS;
  return (w.options || []).filter(o => table[o]).map(o => btn(`data-sess-gate="${esc(o)}"`, table[o])).join('');
}

/* Redrawn only when what it shows changed, and not while a comment is being typed: replacing
   the box under the cursor cuts an IME composition short. `force` is for the banner's own
   answer, which has to show its error. */
function renderSessionGate(s, force = false) {
  const box = sessEl('sess-gate');
  const w = s?.waiting || null;
  if (!w) {
    box.hidden = true;
    box.innerHTML = '';
    delete box.dataset.gateId;
    sessView.gateSig = '';
    return;
  }
  const folded = !!prefs.sessionsGateFolded;
  const error = sessView.gateError?.id === w.id ? sessView.gateError.text : '';
  const busy = sessBusy.has('gate');
  const since = stampSecs(w.openedAt);
  const mins = since != null && state.now != null ? Math.max(0, Math.floor((state.now - since) / 60)) : null;
  const sig = JSON.stringify([s.id, w.id, w.count, w.kind, folded, error, busy, mins]);
  if (!force && sig === sessView.gateSig && !box.hidden) return;
  const typing = box.contains(document.activeElement) && document.activeElement.tagName === 'TEXTAREA';
  if (typing && !force) return;
  sessView.gateSig = sig;
  const inline = INLINE_GATES.includes(w.kind) || !!w.choices?.length;
  const draft = sessView.gateDraft?.id === w.id ? sessView.gateDraft.text : '';
  const head = `<div class="sess-gate-head">
      <span class="material-symbols-outlined" aria-hidden="true">help</span>
      <span class="sess-gate-kind">${esc(gateKindLabel(w.kind))}</span>
      <span class="sess-gate-title">${esc(w.title || '')}</span>
      <span class="sess-gate-meta">${mins == null ? '' : esc(`${minutesLabel(mins)}待っています`)}${w.count > 1 ? esc(` ・ ほか ${w.count - 1} 件`) : ''}</span>
      <button type="button" class="btn-m3-text sess-gate-fold" data-sess-gate-fold aria-expanded="${!folded}">${folded ? '開く' : '畳む'}</button>
    </div>`;
  box.hidden = false;
  box.dataset.gateId = w.id;
  box.classList.toggle('folded', folded);
  if (folded) { box.innerHTML = head; return; }
  box.innerHTML = head +
    (w.focus ? `<p class="sess-gate-focus">${esc(w.focus)}</p>` : '') +
    (inline ? `<textarea class="sess-gate-comment" aria-label="コメント" placeholder="コメント（修正の指示や質問はここに）">${esc(draft)}</textarea>` : '') +
    (error ? `<p class="sess-gate-error" role="alert">${esc(error)}</p>` : '') +
    `<div class="sess-gate-buttons">${inline ? gateButtonsHtml(w, busy) : ''}
      <button type="button" class="btn-m3-tonal sess-act" data-sess-gate-open><span class="material-symbols-outlined" aria-hidden="true">open_in_new</span><span>レビュー画面で開く</span></button>
    </div>
    <div class="sess-gate-foot">ターミナルで答えても構いません。エージェントが再開すると、この確認待ちは「ターミナルで答えた」として閉じます</div>`;
}

/* The gate lives on the board of the hub that opened it, which is not always this page's. */
async function answerSessionGate(s, decision, choice) {
  const w = s?.waiting;
  if (!w || sessBusy.has('gate')) return;
  // The banner holds off redrawing while a comment is typed, so it can show an older gate than
  // the session waits on now: an answer to that one must not land on this one.
  const shown = sessEl('sess-gate').dataset.gateId;
  if (shown && shown !== w.id) {
    const typed = sessEl('sess-gate').querySelector('.sess-gate-comment')?.value;
    if (typed) sessView.gateDraft = { id: w.id, text: typed };
    sessView.gateError = { id: w.id, text: '確認待ちが変わりました。内容を確かめてから答えてください' };
    renderSessionGate(s, true);
    return;
  }
  const closing = decision === 'close';
  const box = sessEl('sess-gate').querySelector('.sess-gate-comment');
  const comment = (box ? box.value : sessView.gateDraft?.id === w.id ? sessView.gateDraft.text : '').trim();
  const line = closing ? `adj gate close --id ${w.id}`
    : `adj gate answer --id ${w.id} --decision ${decision}` + (choice ? ` --choice ${choice}` : '');
  const fail = text => {
    sessView.gateError = { id: w.id, text };
    note(line, true, text);
    renderSessionGate(currentSession(), true);
    sessEl('sess-gate').querySelector('.sess-gate-comment')?.focus();
  };
  // Sending back or asking without saying what gives the worker nothing to act on.
  if (NEEDS_COMMENT.includes(decision) && !comment) return fail('コメントに内容を書いてください');
  sessBusy.add('gate');
  sessView.gateError = null;
  renderSessionGate(s, true);
  try {
    const base = state.resident ? `/b/${w.slug}` : BASE;
    const res = await fetch(`${base}/api/gates/${enc(w.id)}`, {
      method: 'POST',
      headers: { 'X-Adjutant-Token': TOKEN, 'Content-Type': 'application/json' },
      body: JSON.stringify(closing ? { decision, comment } : { decision, choice, comment }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw Object.assign(new Error(data.error || `${res.status}`), { status: res.status });
    const toHub = s.kind === 'hub' || HUB_KINDS.includes(w.kind);
    note(line, false, closing ? 'worker への通知なしでアーカイブしました' : toHub
      ? 'hub の受信箱に送信' + handedNote({ present: data.present, woken: data.woken })
      : `${(s.worktree || '').split('/').pop()} の outbox に追記` +
        (data.woken ? ' → worker に通知しました' : data.present ? ' → worker は次回の outbox 確認時に読み込みます'
                                                               : ' → worker は停止中のため、回答は outbox で保持されます'));
    sessView.gateDraft = null;
    showSessNotice(closing ? '解決済みとして閉じました' : '回答を送りました');
    await refresh();
  } catch (e) {
    sessBusy.delete('gate');
    return fail(e.status === 404 || e.message === 'no such board' ? 'この hub のボードが見つかりません' : e.message);
  }
  sessBusy.delete('gate');
  renderSessionGate(currentSession(), true);
}

function openGateInReview(s) {
  const w = s?.waiting;
  if (!w) return;
  // The review queue reads every board, so the gate is there whichever board it is on.
  goToGate(w.id, w.slug);
}

/* ── The panel over a session that is not running ── */
function overHtml(s) {
  if (!s) return '';
  const st = restingState(s);
  if (st !== 'stopped' && st !== 'ended') return '';
  const m = sessView.mounted;
  if (m && m.ended == null) return '';
  const button = b => actionButtonHtml(b, { busy: sessBusy.size > 0 });
  if (s.kind === 'hub') {
    const hub = hubActionOf(s);
    const h = (state.hubs || []).find(x => x.id === s.id);
    const starting = !!h && hubStartingNow(h);
    // A stopped hub can be started on a new conversation too, whether it is offered to be
    // started or (a parent hub nobody reports to) to be closed.
    const canFresh = !!h && (hub?.act === 'hub-start' || (hub?.act === 'hub-close' && !!h.key));
    const freshWhy = !h ? '' : hubStartWhy(h) || (starting ? 'hub を起動しています' : '');
    const fresh = canFresh
      ? button({ act: 'hub-reset', icon: 'fiber_new', label: 'hub をリセット…', title: freshWhy || 'hub をリセット：今の会話を引き継がず、新しい会話で hub を起動します（adj hub --new）', disabled: !!freshWhy }) : '';
    const head = st === 'ended' ? 'この hub は役目を終えています' : starting ? 'hub を起動しています' : 'hub は止まっています';
    return `<div class="sess-over-head">${head}</div>` +
      (hub ? `<div class="sess-over-buttons">${button(hub)}${fresh}</div>` : '');
  }
  const shot = sessView.screens[s.id];
  let last;
  // A terminal that ended is still on screen above the panel: a copy of its lines would only
  // cover them.
  if (m) {
    last = '';
  } else if (shot?.lines.length) {
    const mins = state.now != null && shot.at != null ? Math.max(0, Math.floor((state.now - shot.at) / 60)) : null;
    last = `<div class="sess-meta">このページで最後に見た画面${mins == null ? '' : `（${esc(minutesLabel(mins))}前）`}</div><pre class="sess-last-out">${esc(shot.lines.join('\n'))}</pre>`;
  } else {
    const phaseAt = s.phaseAt != null && state.now != null ? `（${minutesLabel(Math.max(0, Math.floor((state.now - s.phaseAt) / 60)))}前）` : '';
    last = `<div class="sess-meta">最後の出力はこのページでは見ていません。${s.phase ? `フェーズ: ${esc(s.phase)}${esc(phaseAt)}。` : ''}最後の出力: ${esc(lastOutputText(s) || '不明')}</div>`;
  }
  let action;
  if (canResume(s)) action = `<div class="sess-over-buttons">${button({ act: 'resume', icon: 'restart_alt', label: '再開', title: '保存された会話を tmux の新しいウィンドウで再開します' })}</div>`;
  else if (!s.conversation) action = '<div class="sess-meta">保存された会話がないため再開できません</div>';
  else action = `<div class="sess-meta">${esc(state.sessionResume?.reason || 'ボードからは再開できません')}</div>`;
  return `<div class="sess-over-head">${st === 'ended' ? 'セッションは終了しています' : 'セッションは止まっています'}</div>${last}${action}`;
}

/* ── Clean up a worktree ── */
let cleanupTarget = null;

function cleanupFactsHtml(s, git) {
  const items = [];
  if (s.present) items.push(['ok', 'セッションが動いています。片付けると閉じます']);
  const u = git.uncommitted || {};
  if (u.files > 0) items.push(['warn', `未コミットの変更: ${u.files} ファイル（+${u.insertions} -${u.deletions}）`]);
  if (u.untracked > 0) items.push(['warn', `追跡されていないファイル: ${u.untracked} 件`]);
  const up = git.unpushed || {};
  if (up.count > 0) {
    const commits = (up.commits || []).slice(0, 5).map(c => `${c.sha} ${c.subject}`).join('、');
    items.push(['warn', `push されていないコミット: ${up.count} 件${commits ? `（${commits}）` : ''}`]);
  }
  const m = git.merged || {};
  items.push(['ok', m.merged === true ? `${m.ref || m.base} にマージ済みです`
    : m.merged === false ? `${m.ref || m.base || 'ベース'} にはマージされていません`
      : `マージ済みかは判定できません${m.reason ? `（${m.reason}）` : ''}`]);
  if (!items.some(([k]) => k === 'warn')) items.unshift(['ok', '失われる作業はありません']);
  return items.map(([k, t]) => `<li class="${k}">${esc(t)}</li>`).join('');
}

async function openCleanupDialog(s) {
  const name = (s.worktree || '').split('/').filter(Boolean).pop() || s.id;
  cleanupTarget = { s, name };
  sessEl('cleanup-name').textContent = name;
  sessEl('cleanup-path').textContent = s.worktree;
  sessEl('cleanup-force-name').textContent = name;
  sessEl('cleanup-confirm').placeholder = name;
  sessEl('cleanup-confirm').value = '';
  sessEl('cleanup-force-run').disabled = true;
  sessEl('cleanup-run').disabled = false;
  const force = sessEl('cleanup-force');
  force.hidden = true;
  force.open = false;
  cleanupError('');
  sessEl('cleanup-facts').innerHTML = '<li>確認しています…</li>';
  const dialog = sessEl('cleanup-dialog');
  dialog.returnValue = '';
  dialog.showModal();
  try {
    const git = await api(`/api/sessions/${enc(s.id)}/git`);
    if (cleanupTarget?.s !== s) return;
    sessEl('cleanup-facts').innerHTML = cleanupFactsHtml(s, git);
  } catch (e) {
    if (cleanupTarget?.s !== s) return;
    sessEl('cleanup-facts').innerHTML = `<li class="warn">${esc(`git の状態を確認できませんでした: ${e.message}`)}</li>`;
  }
}

function cleanupError(text) {
  const el = sessEl('cleanup-error');
  el.hidden = !text;
  el.textContent = text || '';
}

async function runCleanup(forced) {
  const t = cleanupTarget;
  if (!t || sessBusy.has('cleanup')) return;
  const line = `worktree を片付ける (${t.name})` + (forced ? ' 強制' : '');
  sessBusy.add('cleanup');
  sessEl('cleanup-run').disabled = true;
  sessEl('cleanup-force-run').disabled = true;
  cleanupError('');
  try {
    const data = await api(`/api/sessions/${enc(t.s.id)}/cleanup`, {
      method: 'POST', body: JSON.stringify(forced ? { force: true, confirm: sessEl('cleanup-confirm').value } : {}),
    });
    if (!data.removed) {
      const reasons = (data.reasons || []).map(r => `<li class="warn">${esc(r.detail)}</li>`).join('');
      sessEl('cleanup-facts').innerHTML = (data.git ? cleanupFactsHtml(t.s, data.git) : '') + reasons;
      cleanupError('失われる作業があるため、削除しませんでした。' + (data.closed ? 'セッションは閉じました。' : ''));
      const force = sessEl('cleanup-force');
      force.hidden = false;
      force.open = true;
      note(line, false, '失われる作業があるため削除しませんでした');
      return;
    }
    sessEl('cleanup-dialog').close('done');
    const branch = data.branch?.name
      ? (data.branch.deleted ? `ブランチ ${data.branch.name} も削除しました` : `ブランチ ${data.branch.name} は消せませんでした: ${data.branch.error}`) : '';
    const badHooks = (data.hooks || []).filter(h => !h.ok).length;
    const text = ['worktree を片付けました', branch, (data.tasks || []).length ? `タスク ${data.tasks.length} 件を完了にしました` : '',
      badHooks ? `onWorktreeRemove が ${badHooks} 件失敗しました` : '', (data.taskErrors || []).length ? 'タスクの更新に失敗したものがあります' : '']
      .filter(Boolean).join('。');
    const bad = !!branch && !data.branch.deleted || badHooks > 0 || (data.taskErrors || []).length > 0;
    note(line, bad, text);
    showSessNotice(text, bad);
    await refresh();
  } catch (e) {
    cleanupError(e.message);
    note(line, true, e.message);
  } finally {
    sessBusy.delete('cleanup');
    sessEl('cleanup-run').disabled = false;
    sessEl('cleanup-force-run').disabled = sessEl('cleanup-confirm').value !== cleanupTarget?.name;
  }
}
sessEl('cleanup-run').addEventListener('click', () => runCleanup(false));
sessEl('cleanup-force-run').addEventListener('click', () => runCleanup(true));
sessEl('cleanup-confirm').addEventListener('input', e => {
  sessEl('cleanup-force-run').disabled = !cleanupTarget || e.target.value !== cleanupTarget.name;
});
// Enter in the box would submit the dialog's own form, which closes it.
sessEl('cleanup-confirm').addEventListener('keydown', e => {
  if (e.key !== 'Enter' || e.isComposing || e.keyCode === 229) return;
  e.preventDefault();
  if (cleanupTarget && e.target.value === cleanupTarget.name) runCleanup(true);
});
sessEl('cleanup-dialog').addEventListener('close', () => { cleanupTarget = null; });

/* One listener for everything drawn into the main column: the bar, the menu, the banner, the
   panel over the terminal. */
document.querySelector('.sess-main').addEventListener('click', e => {
  const s = currentSession();
  if (e.target.closest('[data-sess-gate-fold]')) {
    prefs.sessionsGateFolded = !prefs.sessionsGateFolded;
    savePrefs();
    return renderSessionGate(s, true);
  }
  if (e.target.closest('[data-sess-gate-open]')) return openGateInReview(s);
  const gate = e.target.closest('[data-sess-gate]');
  if (gate && !gate.disabled) {
    const decision = gate.dataset.sessGate;
    return answerSessionGate(s, decision, gate.dataset.choice);
  }
  const act = e.target.closest('[data-sess-act]');
  if (act && !act.disabled && s) {
    closeSessMenu();
    runSessionAction(act.dataset.sessAct, s);
  }
});
sessEl('sess-gate').addEventListener('input', e => {
  const w = currentSession()?.waiting;
  if (w && e.target.matches('.sess-gate-comment')) sessView.gateDraft = { id: w.id, text: e.target.value };
});
sessEl('sess-menu-btn').addEventListener('click', () => {
  const menu = sessEl('sess-menu');
  menu.hidden = !menu.hidden;
  sessEl('sess-menu-btn').setAttribute('aria-expanded', String(!menu.hidden));
});
document.addEventListener('click', e => { if (!e.target.closest('.sess-menu-wrap')) closeSessMenu(); });
document.addEventListener('keydown', e => { if (e.key === 'Escape') closeSessMenu(); });

/* Start the hub of a group: from the board it is on, which for 「すべて」 is not this page's. */
function startGroupHub(g) {
  if (!scopeAll()) return hubStart(g.id);
  const row = hubBoardOf(g);
  const slug = row?.slug || g.slug;
  return hubStartAt(`/b/${slug}`, g.hub.id, g.hub.key, row).then(() => refresh(true));
}

sessEl('sess-groups').addEventListener('click', e => {
  const act = e.target.closest('[data-hub-act]');
  if (act) {
    if (act.disabled) return;
    const g = sessionGroups().find(x => x.gid === act.closest('.sess-group')?.dataset.gid);
    if (!g) return;
    if (act.dataset.hubAct === 'hub-start') return startGroupHub(g);
    return g.hubSession && openSessionRef(sessionRef(g.hubSession));
  }
  const row = e.target.closest('[data-sref]');
  if (row) openSessionRef(row.dataset.sref);
});
// `toggle` does not bubble, and the list is rebuilt: heard on the way down.
sessEl('sess-groups').addEventListener('toggle', e => {
  const gid = e.target.dataset?.orphans;
  if (gid == null) return;
  const key = `orphans:${gid}`;
  const set = new Set(prefs.sessionsFolded);
  // Drawing the list sets `open` too, which lands here with nothing to change.
  if (set.has(key) === e.target.open) return;
  if (e.target.open) set.add(key); else set.delete(key);
  prefs.sessionsFolded = [...set];
  savePrefs();
}, true);
// The notice sits above the list and the terminal both, outside the main column.
sessEl('sess-notice').addEventListener('click', e => { if (e.target.closest('[data-sess-dismiss]')) showSessNotice(''); });
sessEl('sess-back').addEventListener('click', returnFromSessions);
sessEl('sess-reconnect').addEventListener('click', connectSelected);
