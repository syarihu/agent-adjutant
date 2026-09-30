/* The token arrives in the URL (that is how this page was opened) and travels back in a
   header — which is also the half of the CSRF defence a cross-site form cannot reproduce. */
const TOKEN = new URLSearchParams(location.search).get('token') || '';

/* Where this page's board lives: the root for a board served on its own, and `/b/<slug>` when
   the resident server serves it. Every call the page makes is relative to it. */
const BASE = location.pathname.replace(/\/(index\.html)?$/, '');

let state = { tasks: [], workers: [], pending: [], gates: [] };
let view = 'board';
let log = [];
let selectedTaskId = null;

const PREF_KEY = 'adj-board-split';
// sessionsRail: null follows the window's width until the person chooses 'open' or 'collapsed';
// sessionsFolded holds the hub ids folded away, and 'orphans' while that group is open;
// sessionsSide is the detail sidebar's choice, kept only where the window has room for it.
const prefs = Object.assign({ layout:'tabs', arrange:'top', tab:'human', sessionsRail:null, sessionsFilter:'all', sessionsFolded:[], sessionsSide:'open' },
  (() => { try { return JSON.parse(localStorage.getItem(PREF_KEY)) || {}; } catch { return {}; } })());
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
  document.querySelectorAll('[data-tab]').forEach(b => {
    if (b.dataset.tab === prefs.tab && view === 'board') b.setAttribute('aria-current', 'page');
    else b.removeAttribute('aria-current');
  });
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
  if (view !== 'board') setView('board');
  if (prefs.tab !== board) { prefs.tab = board; applyLayout(); }
  if (prefs.layout === 'split') {
    const pane = document.getElementById(`pane-${board}`);
    if (pane) {
      pane.scrollIntoView({ behavior:'smooth', block:'nearest', inline:'nearest' });
      pane.classList.remove('flash-pane'); void pane.offsetWidth; pane.classList.add('flash-pane');
    }
  }
};
window.jump = function(board, id) {
  if (view !== 'board') setView('board');
  if (prefs.tab !== board) { prefs.tab = board; applyLayout(); }
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
const openGate = (t, data = state) => (data.gates || []).find(g => g.task === t.id);

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
  const currentIds = new Set(list.map(g => g.id));
  if (seenGateIds === null) {
    seenGateIds = currentIds;
    return;
  }
  if (window.Notification && Notification.permission === 'granted') {
    for (const g of list) {
      if (!seenGateIds.has(g.id)) {
        const [label] = kindOf(g.kind);
        const wtName = g.worktree ? g.worktree.split('/').pop() : '';
        const n = new Notification(`【${label}】${g.title}`, {
          body: wtName ? `${wtName} から確認依頼が届きました` : '確認依頼が届きました',
          tag: 'gate-' + g.id,
        });
        n.onclick = () => {
          window.focus();
          judgeGate(g.id);
        };
      }
    }
  }
  seenGateIds = currentIds;
}

async function refresh(force = false) {
  try {
    const next = await api('/api/state');
    checkNewGates(next.gates);
    // Compared without the server's clock, which changes on every poll: with it in, every
    // refresh redrew the page, and a comment being typed into a gate lost its IME
    // composition every two seconds. The clock only moves the elapsed times on the board, so
    // the board is redrawn for it once a minute. Its one box to type into, the instruction in
    // the side sheet, is kept across a redraw (renderHandForm).
    const { now, ...rest } = next;
    const nextJson = JSON.stringify(rest);
    const minute = Math.floor((now || 0) / 60);
    const clockOnly = nextJson === lastStateJson && minute !== lastMinute && (view === 'board' || view === 'sessions');
    if (!force && nextJson === lastStateJson && !clockOnly) return;
    lastStateJson = nextJson;
    lastMinute = minute;
    state = next;
    if (window.__from) return;
    render();
  } catch (e) {
    note(`状態を取得できませんでした: ${e.message}`, true);
  }
}

function render() {
  document.getElementById('repo').textContent = state.repo || '';
  const hub = state.hub || {};
  const hubEl = document.getElementById('hub');
  hubEl.className = 'state ' + (hub.present ? 'good' : hub.stale ? 'bad' : 'warn');
  hubEl.innerHTML = hub.present
    ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-success);">check_circle</span><span>hub 稼働中</span>'
    : hub.stale ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-error);">error</span><span>hub の記録が残っているが止まっている</span>'
                : '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-outline);">radio_button_unchecked</span><span>hub は止まっている</span>';

  renderHubRows();

  const waiting = (state.pending || []).length;
  document.getElementById('inbox').innerHTML = waiting
    ? `<span class="material-symbols-outlined" style="font-size:14px;vertical-align:text-bottom;">mail</span><span>受信箱 ${waiting} 件(hub が未読)</span>` : '';

  document.body.classList.toggle('ide-unset', !ideReady());

  renderColumns();

  const humanItems = (state.tasks || []).filter(t => humanColOf(t));
  const gateTaskIds = new Set((state.tasks || []).map(t => t.id));
  const standaloneGates = (state.gates || []).filter(g => !g.task || !gateTaskIds.has(g.task));
  const totalHuman = humanItems.length + standaloneGates.length;

  const humanBadge = document.getElementById('human-badge');
  if (humanBadge) {
    humanBadge.textContent = totalHuman;
    humanBadge.classList.toggle('zero', !totalHuman);
  }

  const mine = (state.gates || []).length;
  const gateCount = document.getElementById('gate-count');
  if (gateCount) {
    gateCount.textContent = mine;
    gateCount.classList.toggle('zero', !mine);
  }

  // Count active workers not waiting on human
  const activeWorkers = (state.tasks || []).filter(t => ['dispatched', 'pr'].includes(t.status) && !humanColOf(t)).length;
  const agentCount = document.getElementById('agent-count');
  if (agentCount) {
    agentCount.textContent = activeWorkers;
  }

  // The ball count belongs in the tab title: you should know it is your turn without
  // having to look at the page.
  document.title = (totalHuman ? `(${totalHuman}) ` : '') + 'adj';
  updateNotifyButton();
  renderSessionsRail();
  openPendingSession();
  renderSessionsView();
  renderDrawer();
  redrawReview();
  redrawTaskView();
  applyLayout();
}

