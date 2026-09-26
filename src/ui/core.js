/* The token arrives in the URL (that is how this page was opened) and travels back in a
   header — which is also the half of the CSRF defence a cross-site form cannot reproduce. */
const TOKEN = new URLSearchParams(location.search).get('token') || '';

let state = { tasks: [], workers: [], pending: [], gates: [] };
let view = 'board';
let log = [];
let selectedTaskId = null;

const COLUMNS = [
  { id:'backlog',   label:'Backlog',   hint:'未受付' },
  { id:'queued',    label:'待ち',       hint:'並べ替え可' },
  { id:'working',   label:'進行中',     hint:'' },
  { id:'attention', label:'要対応',     hint:'' },
  { id:'pr',        label:'レビュー中', hint:'' },
  { id:'done',      label:'完了',       hint:'' },
];

/* 要対応 is derived from the gate directory, never stored as a status — so the board reads
   the thing it is describing rather than a second copy of it. */
const openGate = t => (state.gates || []).find(g => g.task === t.id);
/* A queued task with a gate open is the hub asking whether to start it: the ball is the
   person's, so it sits in 要対応 rather than looking like it is simply waiting its turn. */
const columnOf = t => ['queued', 'dispatched', 'pr'].includes(t.status) && openGate(t)
  ? 'attention'
  : t.status === 'dispatched' ? 'working'
  : { backlog:'backlog', queued:'queued', pr:'pr', done:'done', cancelled:null }[t.status];

/* The only human-owned moves. Everything else belongs to the hub and its workers. */
const ALLOWED = { backlog:['queued'], queued:['backlog','queued'] };
const canDrop = (from, to) => (ALLOWED[from] || []).includes(to);

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

/* Missing before the first poll, when no button has been drawn yet: read as configured so a
   state without the key never dims them. */
const ideReady = () => state.ideConfigured !== false;
const ideTitle = () => ideReady() ? 'IDEでworktreeを開く' : 'エディタが未設定です（押すと設定方法を表示します）';

async function api(path, options = {}) {
  const res = await fetch(path, {
    ...options,
    headers: { 'X-Adjutant-Token': TOKEN, 'Content-Type': 'application/json', ...(options.headers || {}) },
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `${res.status}`);
  return data;
}

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
    const clockOnly = nextJson === lastStateJson && minute !== lastMinute && view === 'board';
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

  const waiting = (state.pending || []).length;
  document.getElementById('inbox').innerHTML = waiting
    ? `<span class="material-symbols-outlined" style="font-size:14px;vertical-align:text-bottom;">mail</span><span>受信箱 ${waiting} 件(hub が未読)</span>` : '';

  document.body.classList.toggle('ide-unset', !ideReady());

  renderColumns();

  const mine = (state.gates || []).length;
  document.getElementById('gate-count').textContent = mine;
  // The ball count belongs in the tab title: you should know it is your turn without
  // having to look at the page.
  document.title = (mine ? `(${mine}) ` : '') + 'adj';
  updateNotifyButton();
  renderDrawer();
  redrawReview();
  redrawTaskView();
}

