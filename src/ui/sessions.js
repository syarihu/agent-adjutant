/* The セッション view: every session of the repository in a tree on the left, and the selected
   one's terminal filling the rest. The first half is pure (it reads `state` and returns);
   the second draws and drives the terminal. */

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
const BACK_LABEL = { board: 'ボード', review: '要対応レビュー', task: 'タスク詳細' };

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

function sessionActivity(s) {
  return state.now != null && s.lastActivityAt != null && state.now - s.lastActivityAt >= IDLE_AFTER_SECS
    ? 'idle' : 'working';
}

/* waiting / stopped / idle / working / ended, or none for a worktree that has no session. The
   record says a worker is gone but not why, so a worker that is gone at a phase where its work
   is done reads as ended, as its card does. */
function sessionState(s) {
  if (s.waiting) return 'waiting';
  return restingState(s);
}

/* `sessionState` without the gate: whether the session is running, stopped or finished, which a
   session that waits on a gate is too. The server sets `waiting` for one that is not running. */
function restingState(s) {
  if (s.kind === 'hub') {
    if (s.present) return sessionActivity(s);
    // A parent-task hub none of whose checkouts report to it any more is finished.
    const h = (state.hubs || []).find(x => x.id === s.id);
    return h && h.parent && !h.children ? 'ended' : 'stopped';
  }
  if (s.present) return sessionActivity(s);
  if (s.stale) return FINISHED_PHASES.includes(s.phase) ? 'ended' : 'stopped';
  return s.conversation ? 'ended' : 'none';
}

const matchesFilter = (s, st, filter) =>
  filter === 'attention' ? st === 'waiting' || st === 'stopped'
    : filter === 'running' ? !!s.present
      : true;

/* What a row and the context bar call a session: the hub's name, or the worktree's directory. */
function sessionKey(s) {
  if (s.kind === 'hub') {
    const h = (state.hubs || []).find(x => x.id === s.id);
    return h ? hubShortName(h) : s.id;
  }
  return (s.worktree || '').split('/').filter(Boolean).pop() || s.id;
}

/* What a row and the context bar title a session with: `title` is the repository's name for its
   hub, the parent task's title for a parent-task hub and the task's title for a worker, empty
   while it is not known; `key` is what sits beside it (the hub's key, the worktree's name).
   `ask` lets a worker's task be asked of its own board, which the rows never do themselves. */
function sessionTitle(s, ask = false) {
  if (s.kind === 'hub') {
    const h = (state.hubs || []).find(x => x.id === s.id);
    if (!h) return { key: '', title: '' };
    return { key: h.parent ? h.key || '' : '', title: hubTitle(h) || '' };
  }
  return { key: sessionKey(s), title: s.taskTitle || boardTaskTitle(s, ask) };
}

/* The words of the row's main line: the title with its key beside it, or the key alone while
   there is no title. The key is left out when it would only repeat the text. */
function sessionLabel(s, ask = false) {
  const { key, title } = sessionTitle(s, ask);
  const text = title || sessionKey(s);
  return { tag: key && key !== text ? key : '', text };
}

function lastOutputText(s) {
  if (state.now == null || s.lastActivityAt == null) return null;
  const mins = Math.max(0, Math.floor((state.now - s.lastActivityAt) / 60));
  return mins < 1 ? 'たった今' : `${minutesLabel(mins)}前`;
}

/* The tree: each hub with its workers, sorted by state within a hub. Hubs stay in the order
   the server gives (the repository's first) — sorting them by state would move them under the
   pointer. Workers whose hub is not listed go in a group of their own at the end. */
function sessionTree(filter, pendingHubs = new Set()) {
  const sessions = state.sessions || [];
  const groups = [];
  const byId = new Map();
  const group = (id, hub) => {
    let g = byId.get(id);
    if (!g) {
      g = { id, hub, hubSession: null, rows: [], unknown: !hub };
      byId.set(id, g);
      groups.push(g);
    }
    return g;
  };
  for (const h of state.hubs || []) group(h.id, h);
  const listed = groups.length;
  const orphans = [];
  for (const s of sessions) {
    if (s.kind === 'hub') { group(s.id, null).hubSession = s; continue; }
    const st = sessionState(s);
    if (st === 'none') { orphans.push(s); continue; }
    if (matchesFilter(s, st, filter)) group(hubOfSession(s) || 'hub', null).rows.push({ s, state: st });
  }
  // Unlisted groups after the listed ones, by id.
  const tail = groups.splice(listed).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  groups.push(...tail);
  for (const g of groups) {
    if (g.hub && !g.hubSession) g.hubSession = sessions.find(s => s.kind === 'hub' && s.id === g.id) || null;
    // A hub with no session of its own is only as alive as its record says.
    g.own = g.hubSession || { kind: 'hub', id: g.id, present: !!g.hub?.state?.present };
    g.state = sessionState(g.own);
    g.short = g.hub ? hubShortName(g.hub) : g.id === 'hub' ? 'リポジトリ' : g.id.replace(/^hub-/, '');
    g.title = g.hub ? hubTitle(g.hub) : g.id === 'hub' ? repoName() : null;
    g.text = g.title || g.short;
    // The repository's own hub says only the name; a parent task's key goes beside its title.
    g.tag = g.title && g.hub?.parent && g.hub.key ? g.hub.key : '';
    g.label = g.hub ? hubLabel(g.hub)
      : g.id === 'hub' ? 'リポジトリの hub' : `親タスク ${g.short} の hub（一覧にありません）`;
    g.rows.sort((a, b) =>
      STATE_ORDER[a.state] - STATE_ORDER[b.state]
      || (a.state === 'waiting' ? (stampSecs(a.s.waiting.openedAt) || 0) - (stampSecs(b.s.waiting.openedAt) || 0) : 0)
      || (a.s.id < b.s.id ? -1 : a.s.id > b.s.id ? 1 : 0));
  }
  const shown = groups.filter(g =>
    (g.hub && !g.hub.parent) || g.rows.length || pendingHubs.has(g.id) || matchesFilter(g.own, g.state, filter));
  return { groups: shown, orphans };
}

/* ── Rail item ── */
function renderSessionsRail() {
  const nav = document.getElementById('nav-sessions');
  if (!nav) return;
  const available = !!state.boardTerminal?.available;
  nav.hidden = !available;
  if (!available) return;
  const sessions = state.sessions || [];
  const running = sessions.filter(s => s.present).length;
  const count = document.getElementById('sessions-count');
  count.textContent = running;
  count.classList.toggle('zero', !running);
  const repoHub = (state.hubs || []).find(h => !h.parent);
  const dot = repoHub && !repoHub.state?.present
    || sessions.some(s => s.kind !== 'hub' && sessionState(s) === 'stopped');
  document.getElementById('sessions-dot').hidden = !dot;
}

/* ── The view ── */
const sessView = {
  selectedId: null,
  last: null,        // the selected session as last listed, kept after it drops off the list
  back: null,        // { view, taskId } to return to, until the first switch to another session
  mounted: null,     // { sessionId, term, ended }
  pending: null,     // a deep link, opened once the first poll says whether a terminal exists
  treeStructure: '', // what the tree was last built from (renderSessionTree)
  treeRows: new Map(), // each row's own signature, by `row:<session id>` and `head:<group id>`
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
const narrowRail = matchMedia('(max-width: 1199px)');
const railCollapsed = () => prefs.sessionsRail === 'collapsed' || (prefs.sessionsRail == null && narrowRail.matches);
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
  if (/^#sessions?(\/|$)/.test(location.hash)) history.replaceState(null, '', location.pathname + location.search);
}

function openSessionsView(id, { from } = {}) {
  if (!state.boardTerminal?.available) return;
  if (view === 'sessions') {
    if (id) selectSession(id);
    return;
  }
  // Below 1400px the sidebar floats over the terminal, so it starts out of the way.
  sessView.sideNarrowOpen = false;
  setView('sessions');
  if (id) selectSession(id);
  // After the selection, which clears a link left over from an earlier switch.
  sessView.back = from || null;
  renderSessionContext();
}

/* A link that arrived before the first poll had said whether this board has terminals. */
function openPendingSession() {
  const p = sessView.pending;
  // No terminal key yet means no poll has succeeded; the next one comes back here.
  if (!p || state.boardTerminal === undefined) return;
  sessView.pending = null;
  if (state.boardTerminal?.available) openSessionsView(p.id);
  else if (view !== 'board') setView('board');
}

function selectSession(id) {
  const changed = sessView.selectedId !== id;
  if (changed) {
    sessView.back = null;
    sessView.last = null;
    sessView.git = null;
    sessView.reconnectWhenReady = null;
    sessView.gateError = null;
    showSessNotice('');
    detachSessionTerminal();
  } else if (sessView.mounted?.ended != null) {
    // The same row again is the way to connect once more.
    detachSessionTerminal();
  }
  sessView.selectedId = id;
  history.replaceState(null, '', '#session/' + encodeURIComponent(id));
  renderSessionsView();
}

function connectSelected() {
  detachSessionTerminal();
  renderSessionsView();
}

function returnFromSessions() {
  const back = sessView.back;
  sessView.back = null;
  setView(back?.view || 'board');
  if (back?.taskId && (back.view || 'board') === 'board' && (state.tasks || []).some(t => t.id === back.taskId)) selectTask(back.taskId);
}

/* A worker's task title when the server did not give one. The task of another board is not in
   this page's state: asked of that board once (when `ask`), and remembered. */
function boardTaskTitle(s, ask) {
  if (!s.task) return '';
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
  sessEl('sess-back').hidden = !sessView.back;
  if (sessView.back) sessEl('sess-back').textContent = `← ${BACK_LABEL[sessView.back.view] || 'ボード'}に戻る`;
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
  if (!id) return '左の一覧からセッションを選んでください';
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

/* A row's words, top to bottom: the title (up to two lines), where it works (the branch, or the
   key when there is none), and how it is. A line that would only repeat the title is left out. */
const rowTextHtml = (text, where, sub) =>
  `<span class="sess-row-text"><span class="sess-row-title">${esc(text)}</span>${where && where !== text ? `<span class="sess-row-branch">${esc(where)}</span>` : ''}<span class="sess-row-sub">${esc(sub)}</span></span>`;

/* A row's tooltip: the whole title, where the worktree is, the tab's own title when it says
   something else, and how the session is. */
function sessionTip(s, st) {
  const { title } = sessionTitle(s);
  const where = s.kind === 'hub' ? '' : `${sessionKey(s)}${s.branch ? ` (${s.branch})` : ''}`;
  const sub = st ? [STATE_LABEL[st], s.present ? lastOutputText(s) : null].filter(Boolean).join(' · ') : '';
  return [title, where, s.title && s.title !== title && s.kind !== 'hub' ? s.title : '', sub].filter(Boolean).join('\n');
}

/* A hub's row says the name and tooltip of its group instead of the session's own. */
function sessionRowHtml(s, st, { label = sessionLabel(s), where = s.branch || label.tag, tip = '', cls = '' } = {}) {
  const sub = [STATE_LABEL[st], s.present ? lastOutputText(s) : null].filter(Boolean).join(' · ');
  tip = tip || sessionTip(s, st);
  return `<button type="button" class="sess-row ${cls} ${st}" data-sid="${esc(s.id)}" title="${esc(tip)}">
    <span class="material-symbols-outlined sess-ico" aria-hidden="true">${STATE_ICON[st]}</span>
    ${rowTextHtml(label.text, where, sub)}</button>`;
}

/* A group's head: the hub's own session when it has one, else a row of the group's own. */
function sessionHeadHtml(g) {
  const label = { tag: g.tag, text: g.text };
  // A parent-task hub's key says more than the branch of the checkout it runs in.
  const where = g.tag || g.hubSession?.branch || '';
  const tip = [g.title, g.label].filter(Boolean).join('\n');
  if (g.hubSession) return sessionRowHtml(g.hubSession, g.state, { label, where, tip, cls: 'hub' });
  return `<div class="sess-row hub ${g.state}" data-gid="${esc(g.id)}" title="${esc(tip)}"><span class="material-symbols-outlined sess-ico" aria-hidden="true">${STATE_ICON[g.state]}</span>${rowTextHtml(label.text, where, STATE_LABEL[g.state])}</div>`;
}

const htmlNode = html => {
  const t = document.createElement('template');
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
};

/* `old` replaced by the node `html` makes, keeping what a redraw must not take from a row:
   the selection and the keyboard focus. */
function swapSessionRow(old, html) {
  const node = htmlNode(html);
  const focused = document.activeElement === old;
  if (old.hasAttribute('aria-current')) node.setAttribute('aria-current', 'true');
  old.replaceWith(node);
  if (focused) node.focus({ preventScroll: true });
  return node;
}

function renderSessionTree() {
  const collapsed = railCollapsed();
  const filter = prefs.sessionsFilter;
  // Rows for sessions asked for and not started yet (sessions-start.js), by hub.
  const pend = sessionPendingRows();
  const pendByHub = new Map();
  for (const p of pend) pendByHub.set(p.hubId, [...(pendByHub.get(p.hubId) || []), p]);
  const { groups, orphans } = sessionTree(filter, new Set(pendByHub.keys()));
  const folded = new Set(prefs.sessionsFolded || []);
  // What each row draws, one signature per row, so that a row is redrawn only when its own
  // words change and not whenever any other row's do.
  const rowSig = (s, st, tip = '') => JSON.stringify([s.id, st, s.title || '', s.branch || '', sessionKey(s), sessionLabel(s), sessionTip(s), s.present ? lastOutputText(s) : '', tip]);
  const rows = new Map();
  for (const g of groups) {
    rows.set(`head:${g.id}`, JSON.stringify([g.hubSession ? rowSig(g.hubSession, g.state, g.label) : null, g.id, g.short, g.tag, g.text, g.title, g.label, g.state]));
    for (const r of g.rows) rows.set(`row:${r.s.id}`, rowSig(r.s, r.state));
  }
  // What the tree is made of: a change here is rebuilt. The order of a group's rows is left
  // out, since moving the rows it already has is enough for that.
  const structure = JSON.stringify([
    collapsed, filter, [...folded],
    groups.map(g => [g.id, g.short, !!g.hubSession, !collapsed && folded.has(g.id) && !pendByHub.has(g.id),
      g.rows.map(r => r.s.id).sort()]),
    orphans.map(s => [s.id, sessionKey(s)]),
    // Part of the signature: without it a row that appears or changes its words while nothing
    // else does would stay hidden behind the redraw skip below.
    pend.map(p => [p.key, p.hubId, p.name, p.kind, p.text, p.canStart, p.busy]),
  ]);
  const tree = sessEl('sess-tree');
  if (structure === sessView.treeStructure) {
    patchSessionTree(tree, groups, rows);
    return;
  }
  sessView.treeStructure = structure;
  sessView.treeRows = rows;
  const scroll = document.querySelector('.sess-scroll');
  const top = scroll.scrollTop;
  tree.innerHTML = groups.map(g => {
    // A group with a session being started stays open: its row is what says the request is there.
    const closed = !collapsed && folded.has(g.id) && !pendByHub.has(g.id);
    return `<div class="sess-group">
      <div class="sess-hubrow"><button type="button" class="sess-fold" data-fold="${esc(g.id)}" aria-expanded="${!closed}" aria-label="${esc(g.short)} を${closed ? '開く' : '畳む'}"><span class="material-symbols-outlined" aria-hidden="true">${closed ? 'chevron_right' : 'expand_more'}</span></button>${sessionHeadHtml(g)}</div>
      <div class="sess-children"${closed ? ' hidden' : ''}>${(pendByHub.get(g.id) || []).map(pendingRowHtml).join('')}${g.rows.map(r => sessionRowHtml(r.s, r.state)).join('')}</div>
    </div>`;
  }).join('') || '<div class="sess-empty">条件に合うセッションはありません</div>';
  const details = sessEl('sess-orphans');
  details.hidden = !orphans.length;
  sessEl('sess-orphans-count').textContent = orphans.length;
  details.open = folded.has('orphans');
  sessEl('sess-orphans-list').innerHTML = orphans.map(s => sessionRowHtml(s, 'none', { label: { tag: '', text: sessionKey(s) } })).join('');
  scroll.scrollTop = top;
}

/* The tree as it stands, brought up to `groups` without rebuilding it: only the rows whose own
   signature changed are redrawn, and the rows of a group that came in another order are
   moved. What is on screen keeps its scroll position, its focus and its selection. */
function patchSessionTree(tree, groups, rows) {
  const before = sessView.treeRows;
  const changed = key => before.get(key) !== rows.get(key);
  // Moving a node blurs it, so the focused row is found again afterwards.
  const focused = document.activeElement;
  const focusedSid = focused?.closest?.('#sess-tree') ? focused.dataset?.sid : null;
  groups.forEach((g, i) => {
    const el = tree.children[i];
    const head = el.querySelector(':scope > .sess-hubrow > .sess-row');
    if (changed(`head:${g.id}`)) swapSessionRow(head, sessionHeadHtml(g));
    const box = el.querySelector(':scope > .sess-children');
    const byId = new Map();
    for (const row of box.querySelectorAll(':scope > .sess-row[data-sid]')) byId.set(row.dataset.sid, row);
    let next = box.querySelector(':scope > .sess-row[data-sid]');
    for (const r of g.rows) {
      const old = byId.get(r.s.id);
      let node = old;
      if (changed(`row:${r.s.id}`)) {
        node = swapSessionRow(old, sessionRowHtml(r.s, r.state));
        if (old === next) next = node;
      }
      if (node === next) next = next.nextElementSibling;
      else box.insertBefore(node, next);
    }
  });
  sessView.treeRows = rows;
  if (focusedSid != null && document.activeElement !== focused) {
    const again = focused.isConnected ? focused
      : [...tree.querySelectorAll('.sess-row[data-sid]')].find(row => row.dataset.sid === focusedSid);
    again?.focus({ preventScroll: true });
  }
}

function applySessionSelection() {
  for (const row of document.querySelectorAll('#sessions-view .sess-row[data-sid]')) {
    if (row.dataset.sid === sessView.selectedId) row.setAttribute('aria-current', 'true');
    else row.removeAttribute('aria-current');
  }
}

function renderSessionsView() {
  if (view !== 'sessions') return;
  if (!state.boardTerminal?.available) { setView('board'); return; }
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
  const collapsed = railCollapsed();
  const rail = document.querySelector('.sess-rail');
  rail.classList.toggle('collapsed', collapsed);
  const toggle = sessEl('sess-collapse');
  toggle.setAttribute('aria-expanded', String(!collapsed));
  toggle.title = collapsed ? '一覧を広げる' : '一覧を畳む';
  toggle.querySelector('.material-symbols-outlined').textContent = collapsed ? 'left_panel_open' : 'left_panel_close';
  for (const chip of rail.querySelectorAll('[data-sess-filter]')) {
    const on = chip.dataset.sessFilter === prefs.sessionsFilter;
    chip.classList.toggle('active', on);
    chip.setAttribute('aria-pressed', String(on));
  }
  renderSessionTree();
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

/* What the board can do about a hub, by the same rule as its row in the hub list: a parent-task
   hub that no checkout reports to any more is closed, whether or not it runs. */
function hubActionOf(s) {
  const h = (state.hubs || []).find(x => x.id === s.id);
  if (!h) return null;
  const present = !!(s.present ?? h.state?.present);
  if (h.parent && !h.children) return { act: 'hub-close', icon: 'close', label: 'hub を閉じる', title: 'この hub を止めて一覧から外します。タスク・gate・受信箱の記録は残ります' };
  if (present) return { act: 'hub-stop', icon: 'stop', label: 'hub を止める', title: 'hub が動いている tmux のペインを閉じます' };
  const why = h.parent && !h.key ? 'キーが分からないため起動できません。adj hub --hub <キー> で起動してください'
    : !state.hubStart?.available ? 'ボードからの起動は terminal.preset が "tmux" のときだけ使えます' : '';
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
      return sessAct('hub-start', `hub を起動 (${key})`, async () => {
        const data = await api(`/api/hubs/${id}/start`, { method: 'POST', body: '{}' });
        const text = data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました';
        note(`hub を起動 (${key})`, false, text);
        showSessNotice(text);
        sessView.reconnectWhenReady = s.id;
        await refresh();
      });
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
  if (!state.resident || BASE === `/b/${w.slug}`) goToGate(w.id);
  else location.href = `/b/${w.slug}/?token=${enc(TOKEN)}#gate/${w.id}`;
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
    return `<div class="sess-over-head">${st === 'ended' ? 'この hub は役目を終えています' : 'hub は止まっています'}</div>` +
      (hub ? `<div class="sess-over-buttons">${button(hub)}</div>` : '');
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
  if (e.target.closest('[data-sess-dismiss]')) return showSessNotice('');
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

document.querySelector('.sess-rail').addEventListener('click', e => {
  const collapse = e.target.closest('[data-sess-collapse]');
  if (collapse) {
    prefs.sessionsRail = railCollapsed() ? 'open' : 'collapsed';
    savePrefs();
    return renderSessionsView();
  }
  const chip = e.target.closest('[data-sess-filter]');
  if (chip) {
    prefs.sessionsFilter = chip.dataset.sessFilter;
    savePrefs();
    return renderSessionsView();
  }
  const fold = e.target.closest('[data-fold]');
  if (fold) {
    const set = new Set(prefs.sessionsFolded);
    if (!set.delete(fold.dataset.fold)) set.add(fold.dataset.fold);
    prefs.sessionsFolded = [...set];
    savePrefs();
    return renderSessionsView();
  }
  const row = e.target.closest('[data-sid]');
  if (row) selectSession(row.dataset.sid);
});
sessEl('sess-orphans').addEventListener('toggle', e => {
  const set = new Set(prefs.sessionsFolded);
  // Drawing the tree sets `open` too, which lands here with nothing to change.
  if (set.has('orphans') === e.target.open) return;
  if (e.target.open) set.add('orphans'); else set.delete('orphans');
  prefs.sessionsFolded = [...set];
  savePrefs();
});
sessEl('sess-back').addEventListener('click', returnFromSessions);
sessEl('sess-reconnect').addEventListener('click', connectSelected);
// Until the person has chosen, the tree follows the window: icons only when it is narrow.
narrowRail.addEventListener('change', () => { if (prefs.sessionsRail == null) renderSessionsView(); });
