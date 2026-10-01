/* The token arrives in the URL (that is how this page was opened) and travels back in a
   header — which is also the half of the CSRF defence a cross-site form cannot reproduce. */
const TOKEN = new URLSearchParams(location.search).get('token') || '';

/* Where the selected board lives: the root for a board served on its own, and `/b/<slug>` when
   the resident server serves it. Every call the page makes is relative to it, so it follows the
   selected board: `go()` reassigns it on a board switch, and nothing may keep a copy of it. */
let BASE = (m => m ? `/b/${m[1]}` : '')(/^\/b\/([^/]+)/.exec(location.pathname));

const emptyState = () => ({ tasks: [], workers: [], pending: [], gates: [] });
let state = emptyState();
let view = 'board';
let log = [];
let selectedTaskId = null;   // the task the panel is open on
let panelPop = false;        // the panel is popped out: never saved, so a reload comes back to its side
// The panel's terminal, kept while its task is open: moving the panel or switching to 詳細 must
// not take the socket down. `reconnect` asks the next draw to mount a fresh one (after 再開).
const panelTerm = { taskId: null, sessionId: null, term: null, ended: null, reconnect: false };

const PREF_KEY = 'adj-board-split';
// sessionsFolded holds `orphans:<group>` for each hub whose worktrees without a session are open;
// sessionsSide is the detail sidebar's choice, kept only where the window has room for it;
// boardsFolded holds the repositories (owner/name) whose hubs are folded away in the sidebar;
// panelSide and panelWidth are where the task panel sits (left or right) and how wide it is.
const prefs = Object.assign({ layout:'tabs', arrange:'top', tab:'human', sessionsFolded:[], sessionsSide:'open', boardsFolded:[], panelSide:'left', panelWidth:520 },
  (() => { try { return JSON.parse(localStorage.getItem(PREF_KEY)) || {}; } catch { return {}; } })());
if (prefs.panelSide !== 'right') prefs.panelSide = 'left';
if (!(prefs.panelWidth >= 320)) prefs.panelWidth = 520;
const savePrefs = () => { try { localStorage.setItem(PREF_KEY, JSON.stringify(prefs)); } catch {} };

function applyLayout() {
  const boards = document.getElementById('boards');
  if (!boards) return;
  boards.className = `layout-${prefs.layout} arrange-${prefs.arrange}`;
  document.body.classList.toggle('layout-tabs', prefs.layout === 'tabs');
  document.body.classList.toggle('layout-split', prefs.layout === 'split');
  const ph = document.getElementById('pane-human');
  const pa = document.getElementById('pane-agent');
  if (ph) ph.classList.toggle('active', prefs.tab === 'human');
  if (pa) pa.classList.toggle('active', prefs.tab === 'agent');
  // Side by side, both boards are on screen, so both tabs are lit.
  const split = prefs.layout === 'split';
  document.querySelectorAll('.view-tab[data-tab]').forEach(b => {
    const tab = b.dataset.tab;
    const on = tab === 'sessions' ? nav.view === 'sessions'
      : view === 'board' && nav.view !== 'sessions' && (split || tab === prefs.tab);
    b.classList.toggle('active', on);
    b.setAttribute('aria-selected', String(on));
  });
  // The layout switches belong to the two boards; the review queue and the task view have no tabs.
  const tools = document.getElementById('view-tools');
  if (tools) tools.hidden = view !== 'board' || nav.view === 'sessions';
  const tabsRow = document.getElementById('view-tabs-row');
  if (tabsRow) tabsRow.hidden = view === 'review' || view === 'task';
  document.querySelectorAll('[data-layout]').forEach(b => {
    const on = b.dataset.layout === prefs.layout;
    b.classList.toggle('active', on);
    b.setAttribute('aria-pressed', String(on));
  });
  document.querySelectorAll('[data-arrange]').forEach(b => {
    const on = b.dataset.arrange === prefs.arrange;
    b.classList.toggle('active', on);
    b.setAttribute('aria-pressed', String(on));
  });
  savePrefs();
}
window.setBoardLayout = function(layout) { prefs.layout = layout; applyLayout(); };
window.setBoardArrange = function(arrange) { prefs.arrange = arrange; applyLayout(); };
window.showBoard = function(board) {
  // Side by side both are already on screen: choosing one is choosing to look at it alone. From
  // another tab nothing was on screen to choose between, and the layout is left as it was.
  if (prefs.layout === 'split' && view === 'board' && nav.view !== 'sessions') { prefs.layout = 'tabs'; savePrefs(); }
  // The address says which tab it is (and a board shown on its own page keeps it).
  go({ view: board === 'agent' ? 'agent' : 'human', session: null });
};
window.showSessions = () => go({ view: 'sessions', session: null });
document.querySelector('.view-tabs').addEventListener('click', e => {
  const tab = e.target.closest('.view-tab[data-tab]');
  if (!tab) return;
  if (tab.dataset.tab === 'sessions') showSessions(); else showBoard(tab.dataset.tab);
});
window.jump = function(board, id) {
  const want = board === 'agent' ? 'agent' : 'human';
  if (view !== 'board' || nav.view !== want) go({ view: want });
  const el = document.getElementById(`${board}-${id}`);
  if (!el) return;
  el.scrollIntoView({ behavior:'smooth', block:'nearest', inline:'center' });
  el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash');
};

const AGENT_COLUMNS = [
  { id:'before',     label:'着手前',          icon:'inbox',          hint:'待ち / Backlog' },
  { id:'plan',       label:'計画',            icon:'edit_note',      hint:'' },
  { id:'implement',  label:'実装',            icon:'code',           hint:'' },
  { id:'selfreview', label:'セルフレビュー',  icon:'rule',           hint:'セルフレビュー / 動作確認' },
  { id:'pr',         label:'PR・レビュー対応', icon:'merge',          hint:'PR / レビュー対応 / 報告' },
  { id:'done',       label:'完了',            icon:'check_circle',   hint:'' },
];
const AGENT_COL_OF_PHASE = {
  plan:'plan',
  implement:'implement',
  'self-review':'selfreview',
  verify:'selfreview',
  pr:'pr',
  'pr-bots':'pr',
  review:'pr',
  report:'pr',
};

const HUMAN_COLUMNS = [
  { id:'dispatch', label:'着手確認',          icon:'play_circle',    hint:'始めてよいか / 起票確認' },
  { id:'plan',     label:'計画の承認',        icon:'edit_note',      hint:'進め方を確認' },
  { id:'diff',     label:'差分レビュー',      icon:'difference',     hint:'手元の差分 / push 前の確認' },
  { id:'verify',   label:'動作確認',          icon:'fact_check',     hint:'手で見る項目 / 調査報告' },
  { id:'prreview', label:'PRレビュー',        icon:'merge',          hint:'PR がこちらのボール' },
  { id:'question', label:'質問',              icon:'help',           hint:'worker からの質問' },
];

const COLUMNS = [...AGENT_COLUMNS, ...HUMAN_COLUMNS];

/* 要対応 is derived from the gate directory, never stored as a status — so the board reads
   the thing it is describing rather than a second copy of it. */
const openGate = (t, data = state) => (data.gates || []).find(g => g.task === t.id && (!t._slug || g._slug === t._slug));

const humanLabel = col => (HUMAN_COLUMNS.find(c => c.id === col) || {}).label || col;
const agentLabel = col => (AGENT_COLUMNS.find(c => c.id === col) || {}).label || col;

// A worker that names another task is that task's, whatever worktree a stale record still points
// at; one that names none is a session waiting to be linked, and the worktree joins it.
const workerOf = (t, data = state) => t && t.worktree && (data.workers || []).find(w => w.worktree === t.worktree && (!w.task || w.task === t.id));
const stampSecs = stamp => {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp || '');
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) / 1000 : null;
};
const updatedMs = t => {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(t?.updatedAt || '');
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) : Date.now();
};

function gateHumanCol(kind) {
  switch (kind) {
    case 'dispatch':
    case 'issue':
      return 'dispatch';
    case 'plan':
      return 'plan';
    case 'diff':
      return 'diff';
    case 'verify':
    case 'result':
      return 'verify';
    case 'relay':
      return 'prreview';
    case 'question':
    default:
      return 'question';
  }
}

/* The Rust `board_counts` (src/cmd/serve.rs) counts what waits from the same rules, for the
   sidebar's board rows. Change one and change the other. */
function humanColOf(t, data = state) {
  if (!t) return null;
  const g = openGate(t, data);
  if (g) return gateHumanCol(g.kind);
  if (t.status === 'done' || t.status === 'cancelled') return null;

  // Jules tasks
  if (t.julesSession && t.pr) {
    if (!t.jules?.working && t.jules?.state !== 'FAILED') {
      return 'prreview';
    }
  }

  // Local worker tasks. Only a PR handed to human reviewers (`pr`) waits on a person; one
  // waiting on review bots (`pr-bots`) leaves a person nothing to do.
  const w = workerOf(t, data);
  if (t.pr) {
    if (w) {
      if (w.phase === 'pr') return 'prreview';
    } else if (t.status === 'pr') {
      return 'prreview';
    }
  }
  return null;
}

function agentColOf(t, data = state) {
  if (!t) return 'before';
  if (t.status === 'backlog' || t.status === 'queued') return 'before';
  if (t.status === 'done' || t.status === 'cancelled') return 'done';

  // Jules task mapping
  if (t.executor === 'jules' || t.julesSession) {
    const js = t.jules?.state;
    if (t.pr || js === 'COMPLETED') return 'pr';
    if (['QUEUED', 'PLANNING', 'AWAITING_PLAN_APPROVAL'].includes(js)) return 'plan';
    if (['IN_PROGRESS', 'AWAITING_USER_FEEDBACK', 'PAUSED', 'FAILED'].includes(js)) return 'implement';
    const g = openGate(t, data);
    if (g?.kind === 'plan') return 'plan';
    if (g?.kind === 'diff' || g?.kind === 'verify') return 'selfreview';
    return 'plan';
  }

  // Local worker mapping
  const w = workerOf(t, data);
  if (w && w.phase && AGENT_COL_OF_PHASE[w.phase]) {
    return AGENT_COL_OF_PHASE[w.phase];
  }
  const g = openGate(t, data);
  if (g?.kind === 'plan') return 'plan';
  if (g?.kind === 'diff' || g?.kind === 'verify') return 'selfreview';
  if (t.status === 'pr' || t.pr) return 'pr';
  return 'plan';
}

function waitingSinceMs(t) {
  const g = openGate(t);
  if (g && g.openedAt) {
    const s = stampSecs(g.openedAt);
    if (s) return s * 1000;
  }
  const w = workerOf(t);
  if (w && w.phaseAt) return w.phaseAt * 1000;
  return updatedMs(t);
}

function waitingMinutes(t) {
  const since = waitingSinceMs(t);
  return Math.max(0, Math.floor((Date.now() - since) / 60000));
}

function waitTone(mins) {
  return mins >= 480 ? 't2' : mins >= 60 ? 't1' : 't0';
}

function columnOf(t, data = state) {
  // Human-owned moves and the hand-over form key off the status-level columns.
  if (t && (t.status === 'backlog' || t.status === 'queued')) return t.status;
  const hc = humanColOf(t, data);
  if (hc) return hc;
  return agentColOf(t, data);
}

/* The only human-owned moves. Everything else belongs to the hub and its workers. */
const ALLOWED = { backlog:['queued'], queued:['backlog','queued'] };
const canDrop = (from, to) => (ALLOWED[from] || []).includes(to);

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

/* Missing before the first poll, when no button has been drawn yet: read as configured so a
   state without the key never dims them. */
const ideReady = () => state.ideConfigured !== false;
const ideTitle = () => ideReady() ? 'IDEでworktreeを開く' : 'エディタが未設定です（押すと設定方法を表示します）';

/* `api`, against any board of this server: `base` is that board's root, as `BASE` is this
   page's own. The token is the same for every board on the machine. */
async function boardApi(base, path, options = {}) {
  const res = await fetch(base + path, {
    ...options,
    headers: { 'X-Adjutant-Token': TOKEN, 'Content-Type': 'application/json', ...(options.headers || {}) },
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `${res.status}`);
  return data;
}

const api = (path, options) => boardApi(BASE, path, options);

/* ── Where the page is: one board, every board, or the review queue ──────────────────────
   Navigation state lives in the address, so back/forward and a pasted link land on the same
   screen without a reload; what is only a preference (layout, folded repositories) does not.
     /b/<slug>/?view=agent&task=<id>&pane=term             one board
     /b/<slug>/?view=sessions&session=<id>                 its sessions, one of them open
     /                                                     すべて, every board
     /review?item=<id>                                     要対応レビュー, every board
   A board served on its own has no list of boards, so it is `board: null` at `/`. Every
   address carries `?token=`: the server refuses a GET without it. */
const nav = { board: null, view: 'human', task: null, pane: 'detail', item: null, session: null };
let boards = [];                 // /api/boards: the sidebar's rows
let multiBoard = /^\/b\//.test(location.pathname);   // the resident server: more than one board
let navEpoch = 0;                // bumped on a board switch, so a late answer for the old one is dropped
let navApplying = false;
let pendingTask = null;          // opened once the board's first state is in
let boardJob = null;             // { slug, fn }: run once that board has loaded
const boardStates = {};          // 「すべて」: each board's last state, by slug
const baseOf = x => x?._base || BASE;
const scopeAll = () => multiBoard && nav.board === 'all';

function parseUrl(loc = location) {
  const q = new URLSearchParams(loc.search);
  const out = { board: null, view: 'human', task: q.get('task'), pane: q.get('pane') === 'term' ? 'term' : 'detail', item: q.get('item'), session: null };
  const m = /^\/b\/([^/]+)/.exec(loc.pathname);
  if (loc.pathname === '/review') {
    out.board = multiBoard ? 'all' : null;
    out.view = 'review';
    return out;
  }
  out.board = m ? m[1] : multiBoard ? 'all' : null;
  const v = q.get('view');
  if (v === 'agent' || v === 'sessions') out.view = v;
  // A session id is only its repository's, so 「すべて」 has none to name.
  if (out.view === 'sessions' && out.board !== 'all') out.session = q.get('session');
  // The task panel opens on a board of its own; 「すべて」 switches to the card's board first.
  if (out.board === 'all') out.task = null;
  return out;
}

function urlOf(n = nav) {
  const path = n.view === 'review' ? '/review' : n.board && n.board !== 'all' ? `/b/${n.board}/` : '/';
  let url = path + '?token=' + encodeURIComponent(TOKEN);
  if (n.view === 'agent' || n.view === 'sessions') url += `&view=${n.view}`;
  if (n.session && n.view === 'sessions') url += `&session=${encodeURIComponent(n.session)}`;
  if (n.task && n.view !== 'review') url += `&task=${encodeURIComponent(n.task)}`;
  if (n.task && n.view !== 'review' && n.pane === 'term') url += '&pane=term';
  if (n.item && n.view === 'review') url += `&item=${encodeURIComponent(n.item)}`;
  return url;
}

/* A change of address that is not a move: the card being looked at, the gate being read. */
function setNav(patch) {
  Object.assign(nav, patch);
  history.replaceState(null, '', urlOf());
}

const sameNav = (a, b) => a.board === b.board && a.view === b.view && a.task === b.task && a.item === b.item && a.session === b.session && a.pane === b.pane;

/* Everything that pointed into the board being left. */
function switchBoard() {
  navEpoch++;
  BASE = multiBoard && nav.board && nav.board !== 'all' ? `/b/${nav.board}` : '';
  lastStateJson = '';
  lastMinute = null;
  if (!multiBoard) seenGateIds = null;
  hideTaskPanelState();
  if (typeof detachSessionTerminal === 'function') detachSessionTerminal();
  sessView.selectedId = null;
  sessView.last = null;
  sessView.pending = null;
  boardJob = null;
  // Caches keyed by a bare task id belong to the board being left.
  for (const cache of [histories, openReplies]) for (const k of Object.keys(cache)) delete cache[k];
  historyFailed.clear();
  // 「すべて」 shows what it last read while the new round is on its way; the review queue
  // keeps the gate it was asked for until that round is in (see renderReview).
  allRound = false;
  state = scopeAll() && Object.keys(boardStates).length ? mergeStates(readBoards(), Math.floor(Date.now() / 1000)) : emptyState();
}

function go(patch = {}, { replace = false } = {}) {
  if (boardJob && 'board' in patch && patch.board !== boardJob.slug) boardJob = null;
  const prev = { ...nav };
  Object.assign(nav, patch);
  if (nav.board !== prev.board) {
    if (!('task' in patch)) nav.task = null;
    if (!('item' in patch)) nav.item = null;
  }
  if (nav.view !== 'review') nav.item = null;
  // A session is one of its board's: it does not follow a move to another board or view.
  if (nav.view !== 'sessions' || (nav.board !== prev.board && !('session' in patch))) nav.session = null;
  if (nav.view === 'review' && multiBoard) nav.board = 'all';
  const boardChanged = nav.board !== prev.board;
  if (boardChanged) switchBoard();
  const url = urlOf();
  if (replace || url === location.pathname + location.search) history.replaceState(null, '', url);
  else history.pushState(null, '', url);
  applyNav(boardChanged, onlyPanelMoved(prev));
  askSessionsOfAll(prev.view, boardChanged);
}

/* Only the task panel's card or tab changed: the screen under it is as it was, and drawing it
   again would reconnect the terminal of the セッション tab. */
const onlyPanelMoved = prev => nav.board === prev.board && nav.view === prev.view && nav.session === prev.session
  && nav.item === prev.item && (nav.view === 'sessions' ? view === 'sessions' : nav.view !== 'review' && view === 'board');

/* A move to or from the セッション tab changes what the next poll asks for (the sessions of
   「すべて」, the last lines of a board's), so it is made now. */
function askSessionsOfAll(prevView, boardChanged) {
  if (!boardChanged && (nav.view === 'sessions') !== (prevView === 'sessions')) refresh(true);
}

/* Draw the screen the address names. */
function applyNav(boardChanged, panelOnly = false) {
  const wantTask = nav.task;
  if (!panelOnly) drawScreen(boardChanged);
  // The address names the card that is open: back to one with none closes the panel, back to
  // the card that is open shows its tab, and another card is opened below.
  if (view === 'board' || view === 'sessions') {
    if (!nav.task) { if (selectedTaskId) hideTaskPanelState(); }
    else if (selectedTaskId === nav.task) renderTaskPanel();
  }
  pendingTask = nav.view === 'review' ? null : wantTask;
  if (boardChanged) render();
  if (boardChanged) refresh(true);
  else applyPendingTask();
}

function drawScreen(boardChanged) {
  if (nav.view !== 'sessions') sessView.pending = false;
  navApplying = true;
  try {
    if (nav.view === 'review') {
      if (nav.item) focused = nav.item;
      setView('review');
    } else if (nav.view === 'sessions') {
      if (state.boardTerminal === undefined) {
        // The first poll of this board has not said whether it has terminals.
        sessView.pending = true;
        if (view !== 'board') setView('board');
      } else if (!state.boardTerminal?.available) giveUpSessions();
      else openSessionsView(nav.session);
    } else {
      prefs.tab = nav.view === 'agent' ? 'agent' : 'human';
      if (view !== 'board') setView('board'); else applyLayout();
    }
  } finally { navApplying = false; }
  if (nav.view === 'review') renderReview();
}

/* `final` is a state the board really answered with: a task it does not list is not coming. */
function applyPendingTask(final = false) {
  if (!pendingTask || (view !== 'board' && view !== 'sessions')) return;
  const id = pendingTask;
  if (!(state.tasks || []).some(t => t.id === id)) {
    if (final) pendingTask = null;
    return;
  }
  pendingTask = null;
  if (selectedTaskId !== id) showTaskPanel(id);
}

/* Run `fn` on the board `slug`: now when it is the one shown, else after switching to it and
   after its first state is in, since what `fn` opens is read from that state. */
function onBoard(slug, fn) {
  if (!slug || !multiBoard || (nav.board === slug && nav.view !== 'review')) return fn();
  go({ board: slug, view: nav.view === 'agent' ? 'agent' : 'human' });
  boardJob = { slug, fn };
}
function runBoardJob() {
  if (!boardJob || nav.board !== boardJob.slug) return;
  const { fn } = boardJob;
  boardJob = null;
  fn();
}

window.addEventListener('popstate', () => {
  const next = parseUrl(location);
  boardJob = null;
  if (sameNav(next, nav)) return;
  const boardChanged = next.board !== nav.board;
  const prev = { ...nav };
  Object.assign(nav, next);
  if (boardChanged) switchBoard();
  applyNav(boardChanged, onlyPanelMoved(prev));
  askSessionsOfAll(prev.view, boardChanged);
});

let lastStateJson = '';
let lastMinute = null;
let seenGateIds = null;

function updateNotifyButton() {
  const btn = document.getElementById('btn-notify');
  if (!btn || !('Notification' in window)) {
    if (btn) btn.style.display = 'none';
    return;
  }
  if (Notification.permission === 'granted') {
    btn.innerHTML = '<span class="material-symbols-outlined" style="font-size:16px;">notifications_active</span><span>通知ON</span>';
    btn.title = '確認依頼のデスクトップ通知が有効です';
  } else if (Notification.permission === 'denied') {
    btn.innerHTML = '<span class="material-symbols-outlined" style="font-size:16px;">notifications_off</span><span>通知OFF</span>';
    btn.title = 'ブラウザの設定で通知がブロックされています';
  } else {
    btn.innerHTML = '<span class="material-symbols-outlined" style="font-size:16px;">notifications</span><span>通知を許可</span>';
    btn.title = '確認依頼が届いたときにデスクトップ通知を受け取る';
  }
}

async function toggleNotify() {
  if (!('Notification' in window)) return;
  if (Notification.permission === 'default') {
    const perm = await Notification.requestPermission();
    updateNotifyButton();
    if (perm === 'granted') {
      new Notification('adj', {
        body: '確認依頼が届いたときにデスクトップ通知でお知らせします',
        tag: 'adj-notify-init',
      });
    }
  } else if (Notification.permission === 'denied') {
    alert('ブラウザの設定で通知がブロックされています。ブラウザのアドレスバーのサイト設定から通知を許可してください。');
  }
}

function checkNewGates(gates) {
  const list = gates || [];
  const refOf = g => g._slug ? `${g._slug}/${g.id}` : g.id;
  const currentIds = new Set(list.map(refOf));
  if (seenGateIds === null) {
    seenGateIds = currentIds;
    return;
  }
  if (window.Notification && Notification.permission === 'granted') {
    for (const g of list) {
      if (!seenGateIds.has(refOf(g))) {
        const [label] = kindOf(g.kind);
        const wtName = g.worktree ? g.worktree.split('/').pop() : '';
        const n = new Notification(`【${label}】${g.title}`, {
          body: wtName ? `${wtName} から確認依頼が届きました` : '確認依頼が届きました',
          tag: 'gate-' + refOf(g),
        });
        n.onclick = () => {
          window.focus();
          onBoard(g._slug, () => judgeGate(g.id));
        };
      }
    }
  }
  seenGateIds = currentIds;
}

/* The gates of every board, as /api/boards lists them: with several boards, a gate that opens
   on one that is not shown is still announced. */
const everyGate = () => boards.flatMap(b => (b.gates || []).map(g => ({ ...g, _slug: b.slug })));

let boardsBusy = false;
let boardsAgain = false;
let boardsFetchedAt = 0;
let boardsTimer = null;
const BOARDS_GAP_MS = 5000;
/* The sidebar's list of boards, asked for at most once per BOARDS_GAP_MS however often the
   page wants it: each answer costs the server a `ps` and a `git worktree list` per repository,
   and the selected board's own 2-second poll must stay cheap. A request that comes too soon
   is not lost; it is made when the gap is over. */
function refreshBoards() {
  if (!multiBoard) return Promise.resolve();
  const wait = boardsFetchedAt + BOARDS_GAP_MS - Date.now();
  if (wait <= 0) return fetchBoards();
  if (!boardsTimer) boardsTimer = setTimeout(() => { boardsTimer = null; fetchBoards(); }, wait);
  return Promise.resolve();
}

/* Each board's counts and gates, as /api/boards lists them; it reads records only. */
async function fetchBoards() {
  if (!multiBoard) return;
  if (boardsBusy) { boardsAgain = true; return; }
  boardsBusy = true;
  boardsFetchedAt = Date.now();
  try {
    boards = await boardApi('', '/api/boards');
    checkNewGates(everyGate());
    renderBoardRows();
    renderTitle();
    renderGateCount();
  } catch {
    // The list is a convenience; the board polls keep reporting a server that is gone.
  } finally {
    boardsBusy = false;
    if (boardsAgain) { boardsAgain = false; refreshBoards(); }
  }
}

/* The boards 「すべて」 and the review queue read, in the order the sidebar lists them. A hub
   that is finished has nothing left to show. */
const readBoards = () => boards.filter(b => !b.finished);

let allBusy = false;
let allAgain = false;            // a forced round asked for while one was out
let allRound = false;            // the first full round of this scope has been drawn
let allSkip = false;
let allMinute = null;
/* The boards that answer for their repository on the セッション tab of 「すべて」: its own board,
   which lists the sessions of its parent-task hubs too, else each parent-task board. */
function sessionCarriers(listed) {
  const by = new Map();
  for (const b of listed) by.set(b.nwo, [...(by.get(b.nwo) || []), b]);
  // A repository with no board of its own has only its parent tasks': each lists its own hub,
  // so each is asked.
  return [...by.values()]
    .flatMap(boards => boards.some(b => !b.hub) ? [boards.find(b => !b.hub)] : boards)
    .sort((a, b) => a.nwo.localeCompare(b.nwo));
}

/* 「すべて」 and the review queue: each board's state, one after another, without its sessions.
   Only the セッション tab of 「すべて」 asks for sessions, and only of one board per repository,
   since listing them is the dearest part of a poll.
   A board whose state is unchanged is not redrawn, and nothing is drawn at all unless one is. */
async function refreshAllBoards(force) {
  if (allBusy) {
    // An answer to something just done must not be lost to the round already out.
    if (force) allAgain = true;
    return;
  }
  allBusy = true;
  const epoch = navEpoch;
  try {
    if (!boards.length) await fetchBoards();
    if (epoch !== navEpoch) return;
    const listed = readBoards();
    const carriers = new Set(nav.view === 'sessions' ? sessionCarriers(listed).map(b => b.slug) : []);
    let changed = force;
    let now = 0;
    for (const b of listed) {
      try {
        const next = await boardApi(`/b/${b.slug}`, carriers.has(b.slug) ? '/api/state?lines=1' : '/api/state?sessions=0');
        if (epoch !== navEpoch) return;
        const { now: at, ...rest } = next;
        now = Math.max(now, at || 0);
        // As `refresh` compares them, so a minute turning over does not redraw each board.
        const json = JSON.stringify({ ...rest, sessions: minuteSessions(rest.sessions, at) });
        if (boardStates[b.slug]?.json !== json) changed = true;
        boardStates[b.slug] = { json, data: next };
      } catch (e) {
        if (epoch !== navEpoch) return;
        // A board that cannot answer drops out of the round rather than failing it.
        if (boardStates[b.slug]) { delete boardStates[b.slug]; changed = true; }
      }
    }
    // The person may have moved on while the last board was answering.
    if (epoch !== navEpoch) return;
    const slugs = new Set(listed.map(b => b.slug));
    for (const slug of Object.keys(boardStates)) {
      if (!slugs.has(slug)) { delete boardStates[slug]; changed = true; }
    }
    const minute = Math.floor(now / 60);
    if (!changed && minute === allMinute) return;
    allMinute = minute;
    state = mergeStates(listed, now, carriers);
    allRound = true;
    if (window.__from) return;
    render();
    runBoardJob();
    if (changed) refreshBoards();
  } finally {
    allBusy = false;
    if (allAgain) {
      allAgain = false;
      if (scopeAll()) refreshAllBoards(true);
    }
  }
}

/* One state out of the boards': tasks and gates tagged with the board they came from, workers
   once each (a parent-task hub's board lists its repository's worktrees too). The hubs and
   sessions are the carrier boards' (see sessionCarriers), tagged the same way, since a hub's
   id is only its own repository's: the repository hub of every repository is `hub`. Sessions
   are there only while the セッション tab asks for them. The inbox is left empty. */
function mergeStates(listed, now, carriers = new Set()) {
  const parts = listed.filter(b => boardStates[b.slug]).map(b => ({ slug: b.slug, data: boardStates[b.slug].data }));
  const carried = sessionCarriers(listed).map(b => b.slug).filter(slug => boardStates[slug]);
  const tag = (list, slug) => (list || []).map(x => ({ ...x, _slug: slug, _base: `/b/${slug}` }));
  const seen = new Set();
  const workers = [];
  for (const { slug, data } of parts) {
    for (const w of data.workers || []) {
      if (seen.has(w.worktree)) continue;
      seen.add(w.worktree);
      workers.push({ ...w, _slug: slug, _base: `/b/${slug}` });
    }
  }
  const first = parts[0]?.data || {};
  const lead = boardStates[carried[0]]?.data || first;
  const nwoOf = slug => listed.find(b => b.slug === slug)?.nwo || '';
  return {
    repo: '',
    resident: true,
    tasks: parts.flatMap(p => tag(p.data.tasks, p.slug)),
    gates: parts.flatMap(p => tag(p.data.gates, p.slug)),
    workers,
    // Each repository's carrier board, in the order the tab lists them.
    carriers: carried.map(slug => ({ slug, nwo: nwoOf(slug) })),
    sessions: carried.filter(slug => carriers.has(slug)).flatMap(slug => tag(boardStates[slug].data.sessions, slug)),
    hubs: carried.flatMap(slug => tag(boardStates[slug].data.hubs, slug)),
    pending: [],
    now,
    ideConfigured: first.ideConfigured,
    stuckAfterMinutes: first.stuckAfterMinutes,
    configPath: first.configPath,
    // Whether this round asked for the sessions: otherwise the tab's counts are not known.
    sessionsRead: carried.some(slug => carriers.has(slug)),
    // Whether it asked at all: asked and not read is a failure, not a wait.
    sessionsAsked: carriers.size > 0,
    // What the tab's buttons ask of the server is the same for every board of it.
    boardTerminal: lead.boardTerminal,
    hubStart: lead.hubStart,
    sessionOpen: lead.sessionOpen,
    sessionResume: lead.sessionResume,
    sessionStart: lead.sessionStart,
    hubRunner: lead.hubRunner,
  };
}

/* The sessions as two polls are compared: a session's last activity as the whole minutes it has
   been idle at `now`, which is as fine as the page shows it. `shows` false leaves it out, for the
   views that do not draw it. Clamped at 0: tmux's activity can be a second later than the
   poll's clock, and -1 against 0 between two polls would redraw for nothing. */
function minuteSessions(sessions, now, shows = true) {
  return (sessions || []).map(({ lastActivityAt, ...s }) => !shows || lastActivityAt == null ? s
    : { ...s, lastActivityAt: Math.max(0, Math.floor((now - lastActivityAt) / 60)) });
}

async function refresh(force = false) {
  if (scopeAll()) {
    // Every board is read, so this runs half as often.
    if (!force) {
      allSkip = !allSkip;
      if (allSkip) return;
    }
    return refreshAllBoards(force);
  }
  const base = BASE;
  const epoch = navEpoch;
  try {
    // The last line of each session's pane is read from tmux, so only the tab that shows it asks.
    const next = await boardApi(base, view === 'sessions' ? '/api/state?lines=1' : '/api/state');
    // The person moved to another board while this was on its way.
    if (epoch !== navEpoch) return;
    if (!multiBoard) checkNewGates(next.gates);
    // Compared without the server's clock, which changes on every poll: with it in, every
    // refresh redrew the page, and a comment being typed into a gate lost its IME
    // composition every two seconds. The clock only moves the elapsed times on the board, so
    // the board is redrawn for it once a minute. Its one box to type into, the instruction in
    // the task panel, is kept across a redraw (renderHandForm).
    // A session's last activity is compared by `minuteSessions`: the timestamp itself moves on
    // nearly every poll. The views that do not show it leave it out, so a minute turning over
    // does not redraw them (and cut a comment being typed there); a view switch draws its view
    // afresh.
    const { now, ...rest } = next;
    if (rest.sessions) rest.sessions = minuteSessions(rest.sessions, now, view === 'board' || view === 'sessions');
    const nextJson = JSON.stringify(rest);
    const minute = Math.floor((now || 0) / 60);
    const changed = nextJson !== lastStateJson;
    const clockOnly = !changed && minute !== lastMinute && (view === 'board' || view === 'sessions');
    if (!force && !changed && !clockOnly) return;
    lastStateJson = nextJson;
    lastMinute = minute;
    // A hub that was not running and is now: it has finished starting, whatever the page
    // started it for. Only a fresh state says so, never a redraw of an old one.
    const was = new Set((state.hubs || []).filter(h => h.state?.present).map(h => h.id));
    for (const h of next.hubs || []) if (h.state?.present && !was.has(h.id)) clearHubStarting(h);
    state = next;
    if (window.__from) return;
    render();
    applyPendingTask(true);
    runBoardJob();
    // What changed here changes the counts in the sidebar: read them now rather than at the
    // next tick.
    if (multiBoard && (changed || force)) refreshBoards();
  } catch (e) {
    note(`状態を取得できませんでした: ${e.message}`, true);
  }
}

/* How many things wait on the person in `data`: tasks with the ball, and gates whose task is
   not on the board. */
function waitingIn(data = state) {
  const humanItems = (data.tasks || []).filter(t => humanColOf(t, data));
  const taskIds = new Set((data.tasks || []).map(t => t.id));
  const standalone = (data.gates || []).filter(g => !g.task || !taskIds.has(g.task));
  return humanItems.length + standalone.length;
}

function renderGateCount() {
  const mine = multiBoard ? boards.reduce((n, b) => n + (b.gates || []).length, 0) : (state.gates || []).length;
  const gateCount = document.getElementById('gate-count');
  if (gateCount) {
    gateCount.textContent = mine;
    gateCount.classList.toggle('zero', !mine);
  }
}

function render() {
  document.body.classList.toggle('ide-unset', !ideReady());

  renderColumns();

  const totalHuman = waitingIn();
  const humanBadge = document.getElementById('human-badge');
  if (humanBadge) {
    humanBadge.textContent = totalHuman;
    humanBadge.classList.toggle('zero', !totalHuman);
  }

  renderGateCount();

  // Count active workers not waiting on human
  const activeWorkers = (state.tasks || []).filter(t => ['dispatched', 'pr'].includes(t.status) && !humanColOf(t)).length;
  const agentCount = document.getElementById('agent-count');
  if (agentCount) {
    agentCount.textContent = activeWorkers;
  }

  // The ball count belongs in the tab title: you should know it is your turn without
  // having to look at the page. The board's name follows it, so tabs of several boards differ.
  renderBoardRows();
  renderTitle();
  updateNotifyButton();
  renderSessionsTab();
  openPendingSession();
  renderSessionsView();
  renderTaskPanel();
  redrawReview();
  redrawTaskView();
  applyLayout();
}
