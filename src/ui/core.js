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
let selectedTaskId = null;   // what the panel is open on: a task id, a hub as HUB_REF + its id, or a session with no task as SESS_REF + its id
// The panel's terminal, kept while its task (or hub) is open: moving the panel or switching to 詳細 must
// not take the socket down. `reconnect` asks the next draw to mount a fresh one (after 再開).
const panelTerm = { host: () => document.getElementById('tp-term-host'), redraw: () => renderTaskPanel(), base: () => BASE,
  taskId: null, sessionId: null, term: null, ended: null, reconnect: false };
// The review view's: one terminal for the item on screen, kept across 判断 and ターミナル.
const reviewTerm = { host: () => document.getElementById('rv-term-host'), redraw: () => renderReviewTerm(),
  base: () => baseOf(reviewCurrent()), taskId: null, sessionId: null, term: null, ended: null, reconnect: false };

/* A hub opens in the task panel as `hub:<id>`, where a task opens as its id: in `selectedTaskId`
   and in the address's `task=`. A session with no task opens the same way, as `session:<id>`, until
   it is linked to one: then the panel shows the task. */
const HUB_REF = 'hub:';
const isHubRef = id => typeof id === 'string' && id.startsWith(HUB_REF);
const hubIdOfRef = id => id.slice(HUB_REF.length);
const hubOfRef = (id, data = state) => isHubRef(id) ? (data.hubs || []).find(h => h.id === hubIdOfRef(id)) || null : null;
/* The hub's own session; a hub with none is only as alive as its record says. */
const hubSessionOf = h => (state.sessions || []).find(s => s.kind === 'hub' && s.id === h.id)
  || { kind: 'hub', id: h.id, present: !!h.state?.present };
const SESS_REF = 'session:';
const isSessRef = id => typeof id === 'string' && id.startsWith(SESS_REF);
const sessIdOfRef = id => id.slice(SESS_REF.length);
const sessOfRef = (id, data = state) => isSessRef(id) ? (data.sessions || []).find(s => s.id === sessIdOfRef(id)) || null : null;

const PREF_KEY = 'adj-board-split';
// sessionsFolded holds `orphans:<group>` for each hub whose worktrees without a session are open;
// boardsFolded holds the repositories (owner/name) whose hubs are folded away in the sidebar;
// panelDock is the side the task panel sits on (left or right), panelDialog is whether panels open as a
// dialog instead, and panelWidth is how wide the docked panel is; all three are remembered per browser;
// reviewNext is whether answering in the review view moves on to the next item.
const prefs = Object.assign({ layout:'tabs', arrange:'top', tab:'agent', sessionsFolded:[], boardsFolded:[], panelDock:'right', panelDialog:false, panelWidth:520, reviewNext:true },
  (() => { try { return JSON.parse(localStorage.getItem(PREF_KEY)) || {}; } catch { return {}; } })());
// The old side key is not carried over: savePrefs() writes the whole object, so anyone who ever changed
// a pref has 'left' saved there whether they chose it or not. A new key lets everyone get the right default once.
delete prefs.panelSide;
if (prefs.panelDock !== 'left') prefs.panelDock = 'right';
prefs.panelDialog = prefs.panelDialog === true;
// Not shrunk to the window here: that would be saved back. The panel's max-width bounds it.
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
  // The layout switches belong to the two boards; the review queue has no tabs.
  const tools = document.getElementById('view-tools');
  if (tools) tools.hidden = view !== 'board' || nav.view === 'sessions';
  const tabsRow = document.getElementById('view-tabs-row');
  if (tabsRow) tabsRow.hidden = view === 'review';
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
function setBoardLayout(layout) { prefs.layout = layout; applyLayout(); }
function setBoardArrange(arrange) { prefs.arrange = arrange; applyLayout(); }
function showBoard(board) {
  // Side by side both are already on screen: choosing one is choosing to look at it alone. From
  // another tab nothing was on screen to choose between, and the layout is left as it was.
  if (prefs.layout === 'split' && view === 'board' && nav.view !== 'sessions') { prefs.layout = 'tabs'; savePrefs(); }
  // The address says which tab it is (and a board shown on its own page keeps it).
  go({ view: board === 'agent' ? 'agent' : 'human' });
}
function showSessions() { go({ view: 'sessions' }); }
document.querySelector('.view-tabs').addEventListener('click', e => {
  const tab = e.target.closest('.view-tab[data-tab]');
  if (!tab) return;
  if (tab.dataset.tab === 'sessions') showSessions(); else showBoard(tab.dataset.tab);
});
function jump(board, id) {
  const want = board === 'agent' ? 'agent' : 'human';
  if (view !== 'board' || nav.view !== want) go({ view: want });
  const el = document.getElementById(`${board}-${id}`);
  if (!el) return;
  el.scrollIntoView({ behavior:'smooth', block:'nearest', inline:'center' });
  el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash');
}

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

/* The person's column a card sits in: its open gate's (`humanCol`), else the PR's when the
   server says the PR waits on the person (`waitsOnPerson`). The page puts the two together
   itself, not the server, because it drops a gate it has just answered from a state that was
   already on its way (`dropGate`, `refresh`, `mergeStates`), and the card must leave that
   gate's column at once. */
function humanColOf(t, data = state) {
  if (!t) return null;
  if (t.ownerHub) return t.ownerHub.humanCol || null;
  return openGate(t, data)?.humanCol || (t.waitsOnPerson ? 'prreview' : null);
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
  return minutesSince(since / 1000, Date.now() / 1000);
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

// Shown after a title made from the issue URL, until the issue has been read.
const titlePendingPill = task => task.titlePending
  ? '<span class="m3-pill pill-neutral" style="margin-left:6px;" title="Issue をまだ読めていません">タイトル未取得</span>' : '';

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
    btn.innerHTML = '<span class="material-symbols-outlined" aria-hidden="true" style="font-size:16px;">notifications_active</span><span>通知ON</span>';
    btn.title = '確認依頼と入力待ちのデスクトップ通知が有効です';
  } else if (Notification.permission === 'denied') {
    btn.innerHTML = '<span class="material-symbols-outlined" aria-hidden="true" style="font-size:16px;">notifications_off</span><span>通知OFF</span>';
    btn.title = 'ブラウザの設定で通知がブロックされています';
  } else {
    btn.innerHTML = '<span class="material-symbols-outlined" aria-hidden="true" style="font-size:16px;">notifications</span><span>通知を許可</span>';
    btn.title = '確認依頼が届いたときや、セッションが入力待ちになったときにデスクトップ通知を受け取る';
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
        const wtName = baseName(g.worktree);
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

/* Open the terminal of the session a wait is about: the notification's click, and 要対応's button. */
function openWait(w) {
  onBoard(w._slug, () => openSessionRef(scopeAll() && w._slug ? `${w._slug}/${w.session}` : w.session));
}

let seenWaitKeys = null;
/* A session that has waited on a person for a few seconds, as the server announces it
   (`waits` of /api/state and /api/boards): once per wait, and not for the ones already there
   when the page opened. The server rings the configured notifier itself; this is the page's. */
function checkNewWaits(waits) {
  const list = waits || [];
  const keyOf = w => `${w._slug || ''}/${w.agentSessionId}/${w.since}`;
  const current = new Set(list.map(keyOf));
  if (seenWaitKeys === null) {
    seenWaitKeys = current;
    return;
  }
  if (window.Notification && Notification.permission === 'granted') {
    for (const w of list) {
      // Listed in 要対応 without a ring: the person was at its terminal, or it was up first.
      if (seenWaitKeys.has(keyOf(w)) || w.quiet) continue;
      // The board's own wording (sessions.js), from a session shaped like the ones it reads.
      const asked = { agentSession: { request: w.request } };
      const n = new Notification(`【${permissionLabel(asked)}】${w.name}`, {
        body: w.request ? requestText(asked) : 'ターミナルで入力を待っています',
        tag: 'wait-' + keyOf(w),
      });
      n.onclick = () => {
        window.focus();
        openWait(w);
      };
    }
  }
  seenWaitKeys = current;
}

/* The waits of every board, as /api/boards lists them. */
const everyWait = () => boards.flatMap(b => (b.waits || []).map(w => ({ ...w, _slug: b.slug })));

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
    // A list that was on its way when a gate was answered still has it: ids are claimed fresh,
    // so a ref answered in this page is never a new gate.
    for (const b of boards) {
      const gone = (b.gates || []).filter(g => reviewDone.has(`${b.slug}/${g.id}`));
      if (!gone.length) continue;
      b.gates = b.gates.filter(g => !gone.includes(g));
      b.waiting = Math.max(0, (b.waiting || 0) - gone.length);
    }
    checkNewGates(everyGate());
    checkNewWaits(everyWait());
    renderBoardRows();
    renderTitle();
    renderGateCount();
    // The hub panel reads the list for its origin, address and counts.
    redrawHubPanel();
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
    // The review queue's terminal tab needs the sessions of the one board whose item is shown.
    const rvSlug = nav.view === 'review' && reviewPane === 'term' ? slugOfRef(focused) : null;
    let changed = force;
    let now = 0;
    for (const b of listed) {
      try {
        const next = await boardApi(`/b/${b.slug}`, carriers.has(b.slug) ? '/api/state?lines=1'
          : b.slug === rvSlug ? '/api/state' : '/api/state?sessions=0');
        if (epoch !== navEpoch) return;
        // The rate limits are left out as `now` is: they move on their own, and the view that draws
        // them redraws with the next real change.
        const { now: at, rateLimits, ...rest } = next;
        now = Math.max(now, at || 0);
        // As `refresh` compares them, so a minute turning over does not redraw each board.
        const json = JSON.stringify({ ...rest, sessions: minuteSessions(rest.sessions, at, view) });
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
    state = mergeStates(listed, now, carriers, rvSlug);
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
function mergeStates(listed, now, carriers = new Set(), rvSlug = null) {
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
    gates: parts.flatMap(p => tag(p.data.gates, p.slug)).filter(g => !reviewDone.has(gateRef(g))),
    waits: parts.flatMap(p => tag(p.data.waits, p.slug)),
    parents: parts.flatMap(p => tag(p.data.parents, p.slug)),
    workers,
    // Each repository's carrier board, in the order the tab lists them.
    carriers: carried.map(slug => ({ slug, nwo: nwoOf(slug) })),
    sessions: [...carried.filter(slug => carriers.has(slug)), ...(rvSlug && !carriers.has(rvSlug) && boardStates[rvSlug] ? [rvSlug] : [])]
      .flatMap(slug => tag(boardStates[slug].data.sessions, slug)),
    hubs: carried.flatMap(slug => tag(boardStates[slug].data.hubs, slug)),
    pending: [],
    now,
    // One poll serves every board, so any board's word is the poll's: one that is failing first.
    prPoll: parts.map(p => p.data.prPoll).find(p => p?.error) || parts.map(p => p.data.prPoll).find(Boolean) || null,
    ideConfigured: first.ideConfigured,
    stuckAfterMinutes: first.stuckAfterMinutes,
    configPath: first.configPath,
    // Whether this round asked for the sessions: otherwise the tab's counts are not known.
    sessionsRead: carried.some(slug => carriers.has(slug)),
    // Whether it asked at all: asked and not read is a failure, not a wait.
    sessionsAsked: carriers.size > 0,
    // The board whose sessions were read for the review view's terminal.
    reviewSessionsOf: rvSlug && boardStates[rvSlug] ? rvSlug : null,
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
   been idle at `now`, which is as fine as the page shows it. `mode` is the view being drawn: the
   board and the sessions tab draw the last activity, and others leave it out. The agent's own part
   is cut to what the view draws, since activity and request change on every tool call: the
   sessions tab has all of it, the last message too (`updatedAt` as minutes, `lastEventAt` not drawn), the board only
   the state and, while it waits on a permission prompt, the request its card shows, and any other
   view only the state. In board mode the activity and the last message are left out on purpose,
   though the session card and the task panel draw them: they show with the next real change or the
   minute redraw (a new last message comes with a change of the state or the held state). Clamped
   at 0: tmux's activity can be a second later than the poll's clock, and -1 against 0 between two
   polls would redraw for nothing. */
function minuteSessions(sessions, now, mode = 'sessions') {
  const minutes = secs => Math.max(0, Math.floor((now - secs) / 60));
  const shows = mode === 'board' || mode === 'sessions';
  // The diff, the branch's PR and their errors are read in the background and change on their own;
  // what draws them redraws with the next real change. Sub-agents are compared by how many run,
  // since each one's tool changes on every call.
  return (sessions || []).map(({ lastActivityAt, agentSession: a, uncommitted, uncommittedError, branchPr, branchPrError, ...s }) => {
    const agent = !a ? {} : { agentSession: {
      status: a.status, pending: a.pending, subagents: a.subagents?.length || 0, error: a.error,
      ...(mode === 'sessions' ? { activity: a.activity, request: a.request, lastMessage: a.lastMessage }
        : mode === 'board' && a.status === 'waiting' ? { request: a.request } : {}),
      ...(shows && a.updatedAt != null ? { updatedAt: minutes(a.updatedAt) } : {}),
    } };
    return !shows || lastActivityAt == null ? { ...s, ...agent } : { ...s, ...agent, lastActivityAt: minutes(lastActivityAt) };
  });
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
    // The last line of each session's pane is read from tmux, so only what shows it asks: the
    // sessions tab for every session, the board view for the hub's own.
    const next = await boardApi(base, view === 'sessions' ? '/api/state?lines=1' : view === 'board' ? '/api/state?lines=hub' : '/api/state');
    // The person moved to another board while this was on its way.
    if (epoch !== navEpoch) return;
    if (!multiBoard) {
      checkNewGates(next.gates);
      checkNewWaits(next.waits);
    }
    // Compared without the server's clock, which changes on every poll: with it in, every
    // refresh redrew the page, and a comment being typed into a gate lost its IME
    // composition every two seconds. The clock only moves the elapsed times on the board, so
    // the board is redrawn for it once a minute. Its one box to type into, the instruction in
    // the task panel, is kept across a redraw (renderHandForm).
    // A session's last activity is compared by `minuteSessions`: the timestamp itself moves on
    // nearly every poll. The views that do not show it leave it out, so a minute turning over
    // does not redraw them (and cut a comment being typed there); a view switch draws its view
    // afresh.
    const { now, rateLimits, ...rest } = next;
    if (rest.sessions) rest.sessions = minuteSessions(rest.sessions, now, view);
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
    // Same for a state that was on its way (see fetchBoards).
    if ((next.gates || []).some(g => reviewDone.has(gateRef(g)))) next.gates = next.gates.filter(g => !reviewDone.has(gateRef(g)));
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
  // The sessions waiting on a prompt or a question are in the queue beside the gates.
  const mine = multiBoard
    ? boards.reduce((n, b) => n + (b.gates || []).length + (b.waits || []).length, 0)
    : (state.gates || []).length + (state.waits || []).length;
  const gateCount = document.getElementById('gate-count');
  if (gateCount) {
    gateCount.textContent = mine;
    gateCount.classList.toggle('zero', !mine);
  }
}

/* The header's three counts: what waits on the person, the gates, and the workers at work. */
function renderCounts() {
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
}

/* The parts of the page, in the order `render` draws them and `resetViews` resets them. Each
   script registers its own parts at its end. The order of the resets matters once: the sessions
   tab lets go of the address it waits on before the panel's terminal goes, whose going redraws
   that tab. */
const VIEW_ORDER = ['columns', 'counts', 'board-rows', 'title', 'notify', 'sessions-tab', 'pending-session',
  'sessions-view', 'task-panel', 'review', 'layout'];
const viewsByName = new Map();

/* `render(data)` draws the part (from the page's `state`); `reset()`, where there is one, forgets
   what the part keeps for the board being left. A name not in VIEW_ORDER, or one registered
   twice, throws, so a typo shows at load. */
function registerView(name, part) {
  if (!VIEW_ORDER.includes(name)) throw new Error(`unknown view: ${name}`);
  if (viewsByName.has(name)) throw new Error(`view registered twice: ${name}`);
  viewsByName.set(name, part);
}

function render() {
  document.body.classList.toggle('ide-unset', !ideReady());
  // A part whose script has not registered it yet is skipped.
  for (const name of VIEW_ORDER) viewsByName.get(name)?.render(state);
}

function resetViews() {
  for (const name of VIEW_ORDER) viewsByName.get(name)?.reset?.();
}

registerView('counts', { render: () => renderCounts() });
registerView('notify', { render: () => updateNotifyButton() });
registerView('layout', { render: () => applyLayout() });
