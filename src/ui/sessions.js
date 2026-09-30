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
};
const STATE_ICON = {
  waiting: 'help', stopped: 'error', idle: 'hourglass_empty', working: 'play_circle', ended: 'check_circle', none: 'remove_circle_outline',
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

/* `api`, but against the board the session belongs to. The token is the same for every board
   on the machine. */
async function sessionApi(s, path, options = {}) {
  const res = await fetch(boardOfSession(s).base + path, {
    ...options,
    headers: { 'X-Adjutant-Token': TOKEN, 'Content-Type': 'application/json', ...(options.headers || {}) },
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `${res.status}`);
  return data;
}

function sessionActivity(s) {
  return state.now != null && s.lastActivityAt != null && state.now - s.lastActivityAt >= IDLE_AFTER_SECS
    ? 'idle' : 'working';
}

/* waiting / stopped / idle / working / ended, or none for a worktree that has no session. The
   record says a worker is gone but not why, so a worker that is gone at a phase where its work
   is done reads as ended, as its card does. */
function sessionState(s) {
  if (s.waiting) return 'waiting';
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

function lastOutputText(s) {
  if (state.now == null || s.lastActivityAt == null) return null;
  const mins = Math.max(0, Math.floor((state.now - s.lastActivityAt) / 60));
  return mins < 1 ? 'たった今' : `${minutesLabel(mins)}前`;
}

/* The tree: each hub with its workers, sorted by state within a hub. Hubs stay in the order
   the server gives (the repository's first) — sorting them by state would move them under the
   pointer. Workers whose hub is not listed go in a group of their own at the end. */
function sessionTree(filter) {
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
    g.label = g.hub ? hubLabel(g.hub)
      : g.id === 'hub' ? 'リポジトリの hub' : `親タスク ${g.short} の hub（一覧にありません）`;
    g.rows.sort((a, b) =>
      STATE_ORDER[a.state] - STATE_ORDER[b.state]
      || (a.state === 'waiting' ? (stampSecs(a.s.waiting.openedAt) || 0) - (stampSecs(b.s.waiting.openedAt) || 0) : 0)
      || (a.s.id < b.s.id ? -1 : a.s.id > b.s.id ? 1 : 0));
  }
  const shown = groups.filter(g =>
    (g.hub && !g.hub.parent) || g.rows.length || matchesFilter(g.own, g.state, filter));
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
  treeSig: '',
  taskTitles: {},    // slug -> { [taskId]: title }, for a session of another board
};
const sessEl = id => document.getElementById(id);
const narrowRail = matchMedia('(max-width: 1199px)');
const railCollapsed = () => prefs.sessionsRail === 'collapsed' || (prefs.sessionsRail == null && narrowRail.matches);
const currentSession = () => (state.sessions || []).find(s => s.id === sessView.selectedId) || null;

function detachSessionTerminal() {
  const m = sessView.mounted;
  if (!m) return;
  sessView.mounted = null;
  // Closing the socket only detaches; the session keeps running.
  m.term.dispose();
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

/* A worker that names no title of its own borrows its task's. The task of another board is
   not in this page's state: asked of that board once, and remembered. */
function taskTitleOf(s) {
  if (s.title) return s.title;
  if (!s.task) return '';
  const own = (state.tasks || []).find(t => t.id === s.task);
  if (own) return own.title || '';
  const b = boardOfSession(s);
  if (b.own || !b.slug) return '';
  const known = sessView.taskTitles[b.slug];
  if (known === undefined) {
    sessView.taskTitles[b.slug] = null;
    sessionApi(s, '/api/state').then(other => {
      sessView.taskTitles[b.slug] = Object.fromEntries((other.tasks || []).map(t => [t.id, t.title || '']));
      if (view === 'sessions') renderSessionContext();
    }).catch(() => { sessView.taskTitles[b.slug] = {}; });
    return '';
  }
  return known?.[s.task] || '';
}

function renderSessionContext() {
  const id = sessView.selectedId;
  const s = currentSession() || (id && sessView.last?.id === id ? sessView.last : null);
  const gone = !!id && !currentSession() && sessView.last?.id === id;
  sessEl('sess-context').hidden = !id;
  sessEl('sess-back').hidden = !sessView.back;
  if (sessView.back) sessEl('sess-back').textContent = `← ${BACK_LABEL[sessView.back.view] || 'ボード'}に戻る`;
  if (!id) return;
  const key = s ? sessionKey(s) : id;
  const keyEl = sessEl('sess-key');
  keyEl.textContent = key;
  keyEl.title = s && s.kind !== 'hub' && s.branch ? `ブランチ: ${s.branch}` : '';
  sessEl('sess-title').textContent = s ? taskTitleOf(s) : '';
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
  const st = sessionState(s);
  if (st === 'none') return `セッションはありません。\n${s.worktree}${s.branch ? `（${s.branch}）` : ''}`;
  if (st === 'stopped') return 'セッションは止まっています';
  if (st === 'ended') return 'セッションは終了しています';
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
      onEnd: code => {
        rec.ended = code;
        if (sessView.mounted === rec) renderSessionContext();
      },
    });
  }
  const shown = !!sessView.mounted;
  sessEl('sess-term-host').hidden = !shown;
  const ph = sessEl('sess-placeholder');
  ph.hidden = shown;
  if (!shown) ph.textContent = sessionPlaceholder(s);
}

/* A hub's row says the name and tooltip of its group instead of the session's own. */
function sessionRowHtml(s, st, { key = sessionKey(s), tip = '', cls = '' } = {}) {
  const sub = [STATE_LABEL[st], s.present ? lastOutputText(s) : null].filter(Boolean).join(' · ');
  tip = tip || [key, s.title, sub].filter(Boolean).join('\n');
  return `<button type="button" class="sess-row ${cls} ${st}" data-sid="${esc(s.id)}" title="${esc(tip)}">
    <span class="material-symbols-outlined sess-ico" aria-hidden="true">${STATE_ICON[st]}</span>
    <span class="sess-row-text"><span class="sess-row-key">${esc(key)}</span><span class="sess-row-sub">${esc(sub)}</span></span></button>`;
}

function renderSessionTree() {
  const collapsed = railCollapsed();
  const filter = prefs.sessionsFilter;
  const { groups, orphans } = sessionTree(filter);
  const folded = new Set(prefs.sessionsFolded || []);
  const rowSig = (s, st) => [s.id, st, s.title || '', sessionKey(s), s.present ? lastOutputText(s) : ''];
  const sig = JSON.stringify([
    collapsed, filter, [...folded],
    groups.map(g => [g.id, g.label, g.state, g.hubSession ? rowSig(g.hubSession, g.state) : null,
      g.rows.map(r => rowSig(r.s, r.state))]),
    orphans.map(s => [s.id, sessionKey(s)]),
  ]);
  if (sig === sessView.treeSig) return;
  sessView.treeSig = sig;
  const scroll = document.querySelector('.sess-scroll');
  const top = scroll.scrollTop;
  sessEl('sess-tree').innerHTML = groups.map(g => {
    const closed = !collapsed && folded.has(g.id);
    const head = g.hubSession
      ? sessionRowHtml(g.hubSession, g.state, { key: g.short, tip: g.label, cls: 'hub' })
      : `<div class="sess-row hub ${g.state}" title="${esc(g.label)}"><span class="material-symbols-outlined sess-ico" aria-hidden="true">${STATE_ICON[g.state]}</span><span class="sess-row-text"><span class="sess-row-key">${esc(g.short)}</span><span class="sess-row-sub">${esc(STATE_LABEL[g.state])}</span></span></div>`;
    return `<div class="sess-group">
      <div class="sess-hubrow"><button type="button" class="sess-fold" data-fold="${esc(g.id)}" aria-expanded="${!closed}" aria-label="${esc(g.short)} を${closed ? '開く' : '畳む'}"><span class="material-symbols-outlined" aria-hidden="true">${closed ? 'chevron_right' : 'expand_more'}</span></button>${head}</div>
      <div class="sess-children"${closed ? ' hidden' : ''}>${g.rows.map(r => sessionRowHtml(r.s, r.state)).join('')}</div>
    </div>`;
  }).join('') || '<div class="sess-empty">条件に合うセッションはありません</div>';
  const details = sessEl('sess-orphans');
  details.hidden = !orphans.length;
  sessEl('sess-orphans-count').textContent = orphans.length;
  details.open = folded.has('orphans');
  sessEl('sess-orphans-list').innerHTML = orphans.map(s => sessionRowHtml(s, 'none')).join('');
  scroll.scrollTop = top;
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
  renderSessionContext();
}

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
