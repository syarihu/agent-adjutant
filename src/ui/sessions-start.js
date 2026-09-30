/* Starting hubs and sessions from the セッション view, and giving a session with no task a task:
   the + menu, the three dialogs it and the sidebar open, and the rows the tree shows for a
   session that has been asked for and not started yet. */

const pad2 = n => String(n).padStart(2, '0');
const baseName = path => (path || '').split('/').filter(Boolean).pop() || '';
const repoHubId = () => (state.hubs || []).find(h => !h.parent)?.id || 'hub';
const NO_START = 'ボードからの起動は terminal.preset が "tmux" のときだけ使えます';

/* ── Names ── */

/* Local time here, UTC on the server: the name the dialog shows is the one it sends, so the
   server's own dated name is only what a request with no name gets. */
function datedName(d = new Date()) {
  return `session-${d.getFullYear()}${pad2(d.getMonth() + 1)}${pad2(d.getDate())}-${pad2(d.getHours())}${pad2(d.getMinutes())}`;
}

/* The same four-word rule as `derived_name` in the server (src/cmd/session.rs). */
function proposeName(instruction, d = new Date()) {
  const words = (instruction.match(/[A-Za-z0-9]+/g) || []).slice(0, 4).map(w => w.toLowerCase());
  // Cut at 32 characters like the server's (`derived_name`), with no separator left dangling.
  const name = words.join('-').slice(0, 32).replace(/^-+|-+$/g, '');
  return name && !worktreeNameProblem(name) ? name : datedName(d);
}

/* Why git would refuse the name, or '' when it would not: the server allows only these
   characters, then asks `git check-ref-format`, which adds the rules below. */
function worktreeNameProblem(name) {
  if (!name) return 'worktree 名を入力してください';
  if (/[^A-Za-z0-9._-]/.test(name)) return '使えるのは英数字と . _ - だけです';
  if (name.startsWith('-')) return '先頭を - にはできません';
  if (name.startsWith('.')) return '先頭を . にはできません';
  if (name.endsWith('.')) return '末尾を . にはできません';
  if (name.includes('..')) return '.. は使えません';
  if (name.endsWith('.lock')) return '末尾を .lock にはできません';
  if (name === 'HEAD') return 'HEAD は使えません';
  return '';
}

/* What a worker is called: its worktree's directory, which `worktreePattern` may make
   something other than the name asked for, and its title, which the hub sets to the final name. */
const workerNames = s => [baseName(s.worktree), s.title].filter(Boolean);

function namesInUse() {
  const used = new Set((state.workers || []).map(w => w.name).filter(Boolean));
  for (const s of state.sessions || []) if (s.kind === 'worker') for (const n of workerNames(s)) used.add(n);
  return used;
}

function closeDialogById(id) {
  const d = sessEl(id);
  if (d.open) d.close();
}
for (const b of document.querySelectorAll('[data-dialog-close]')) {
  b.addEventListener('click', () => closeDialogById(b.dataset.dialogClose));
}
/* Not rewritten when unchanged: a live region would announce the same words again. */
const setText = (el, text) => { if (el.textContent !== text) el.textContent = text; };
/* `handed.present` is whether the hub was running when the message arrived, so after a start it
   would say the hub is stopped. */
const startedOrHanded = (started, handed) => started ? ' / hub を起動しました' : handedNote(handed);
const isConfirmKey = e => (e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229;

function showDlgError(id, text) {
  const el = sessEl(id);
  el.hidden = !text;
  el.textContent = text || '';
}

/* Bumped each time a dialog is opened. A request answers after its dialog may have been
   closed and opened again for something else: only a dialog still on the same opening is the
   one to close or to write an error into. */
const dialogOpening = { hubkey: 0, start: 0, link: 0 };
const HUB_STARTING_MS = 15000;

/* ── The + menu ── */
let repoHubStartedAt = null;

function closeAddMenu() {
  sessEl('sess-add-menu').hidden = true;
  sessEl('sess-add').setAttribute('aria-expanded', 'false');
}

function renderAddMenu() {
  const repo = (state.hubs || []).find(h => !h.parent);
  const canStart = !!state.hubStart?.available;
  // A second press right after the first would open a second window.
  const starting = repoHubStartedAt != null && Date.now() - repoHubStartedAt < HUB_STARTING_MS;
  const items = [
    {
      act: 'add-repo-hub', icon: 'play_arrow', label: 'リポジトリの hub を起動',
      disabled: !repo || !!repo.state?.present || !canStart || starting,
      title: repo?.state?.present ? 'リポジトリの hub はすでに動いています' : !canStart ? NO_START
        : starting ? 'hub を起動しています' : 'tmux の新しいウィンドウで adj hub を実行します',
    },
    {
      act: 'add-parent-hub', icon: 'account_tree', label: '親タスクの hub を起動…',
      disabled: !canStart || !state.resident,
      title: !canStart ? NO_START : !state.resident ? 'resident サーバーのボードからだけ使えます' : '',
    },
    { act: 'add-session', icon: 'terminal', label: 'タスクなしのセッションを始める…', title: '' },
  ];
  sessEl('sess-add-menu').innerHTML = items.map(b => actionButtonHtml(b, { menu: true })).join('');
}

sessEl('sess-add').addEventListener('click', () => {
  closeSessMenu();
  const menu = sessEl('sess-add-menu');
  if (menu.hidden) renderAddMenu();
  menu.hidden = !menu.hidden;
  sessEl('sess-add').setAttribute('aria-expanded', String(!menu.hidden));
});
document.addEventListener('click', e => { if (!e.target.closest('.sess-add-wrap')) closeAddMenu(); });
document.addEventListener('keydown', e => {
  if (e.key !== 'Escape' || sessEl('sess-add-menu').hidden) return;
  closeAddMenu();
  sessEl('sess-add').focus();
});
sessEl('sess-add-menu').addEventListener('click', e => {
  const b = e.target.closest('[data-sess-act]');
  if (!b || b.disabled) return;
  closeAddMenu();
  if (b.dataset.sessAct === 'add-repo-hub') return startRepoHub();
  if (b.dataset.sessAct === 'add-parent-hub') return openHubKeyDialog();
  if (b.dataset.sessAct === 'add-session') return openStartDialog();
});

/* A hub that was just started has no session to show yet: select its row, and connect to it
   once the window exists. */
function showStartedHub(id) {
  selectSession(id);
  sessView.reconnectWhenReady = id;
}

function startRepoHub() {
  const repo = (state.hubs || []).find(h => !h.parent);
  if (!repo) return;
  return sessAct('add-hub', 'リポジトリの hub を起動', async () => {
    const data = await api(`/api/hubs/${enc(repo.id)}/start`, { method: 'POST', body: '{}' });
    const text = data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました';
    note('adj hub --tab', false, text);
    showSessNotice(text);
    if (!data.alreadyRunning) repoHubStartedAt = Date.now();
    await refresh(true);
    showStartedHub(repo.id);
  });
}

/* ── A parent task's hub, by key ── */
let hubKeyBusy = false;

function openHubKeyDialog() {
  dialogOpening.hubkey++;
  sessEl('hubkey-key').value = '';
  showDlgError('hubkey-error', '');
  sessEl('hubkey-dialog').showModal();
  setTimeout(() => sessEl('hubkey-key').focus(), 50);
}

async function submitHubKey(e) {
  e.preventDefault();
  if (hubKeyBusy) return;
  const key = sessEl('hubkey-key').value.trim();
  if (!key) return showDlgError('hubkey-error', 'キーを入力してください');
  const opening = dialogOpening.hubkey;
  hubKeyBusy = true;
  sessEl('hubkey-submit').disabled = true;
  showDlgError('hubkey-error', '');
  const line = `adj hub --tab --hub=${key}`;
  try {
    const data = await api('/api/hubs', { method: 'POST', body: JSON.stringify({ key }) });
    const text = data.alreadyRunning ? `親タスク ${key} の hub はすでに動いています` : `親タスク ${key} の hub を tmux で起動しました`;
    note(line, false, text);
    if (opening === dialogOpening.hubkey) closeDialogById('hubkey-dialog');
    showSessNotice(text);
    await refresh(true);
    showStartedHub(data.hub?.id || `hub-${key}`);
  } catch (err) {
    note(`${line} → ${err.message}`, true);
    if (opening === dialogOpening.hubkey) showDlgError('hubkey-error', err.message);
  } finally {
    hubKeyBusy = false;
    sessEl('hubkey-submit').disabled = false;
  }
}
sessEl('hubkey-form').addEventListener('submit', submitHubKey);

/* ── A session with no task ── */
const startDlg = { nameEdited: false, busy: false };

const startableHubs = () => (state.hubs || []).filter(h => !(h.parent && !h.key));
const slotsFull = () => { const w = state.workerSlots; return !!w && w.max != null && w.busy >= w.max; };

function openStartDialog() {
  dialogOpening.start++;
  const hubs = startableHubs();
  const cur = currentSession();
  const want = cur ? hubOfSession(cur) || repoHubId() : repoHubId();
  const sel = sessEl('start-hub');
  sel.innerHTML = hubs.map(h => `<option value="${esc(h.id)}">${esc(hubLabel(h))}</option>`).join('');
  sel.value = hubs.some(h => h.id === want) ? want : hubs[0]?.id || '';
  const agent = state.sessionStart?.agent;
  sessEl('start-agent').innerHTML = agent ? `<option value="${esc(agent)}">${esc(agent)}</option>` : '';
  sessEl('start-instruction').value = '';
  startDlg.nameEdited = false;
  sessEl('start-name').value = proposeName('');
  showDlgError('start-error', '');
  syncStartDialog();
  sessEl('start-dialog').showModal();
  setTimeout(() => sessEl('start-instruction').focus(), 50);
}

/* Everything the dialog says that depends on the state or on what is typed; the typed values
   themselves are left alone, so this can run on every poll. */
function syncStartDialog() {
  const dialog = sessEl('start-dialog');
  if (!dialog.open) return;
  const h = (state.hubs || []).find(x => x.id === sessEl('start-hub').value);
  const hubNote = sessEl('start-hub-note');
  setText(hubNote, !h ? '依頼できる hub がありません'
    : h.state?.present ? 'hub は動いています。依頼はすぐ届きます'
      : state.resident && state.hubStart?.available ? 'hub が止まっているため、依頼を送ったあとで起動します'
        : `hub が止まっていて、ボードからは起動できません（${NO_START}）。依頼は受信箱で、hub が起動するのを待ちます`);

  // A dated name is the time it was proposed, so it is kept current until the person edits it.
  if (!startDlg.nameEdited) {
    const proposed = proposeName(sessEl('start-instruction').value);
    if (sessEl('start-name').value !== proposed) sessEl('start-name').value = proposed;
  }
  const name = sessEl('start-name').value.trim();
  const problem = worktreeNameProblem(name);
  const noteEl = sessEl('start-name-note');
  noteEl.classList.toggle('err', !!problem);
  setText(noteEl, problem
    || (namesInUse().has(name) ? `すでに使われています。hub が ${name}-2 などに変えます` : 'hub がこの名前で worktree とブランチを作ります'));
  sessEl('start-name').setAttribute('aria-invalid', String(!!problem));

  const full = slotsFull();
  const slots = sessEl('start-slots');
  // Reopened while the last request is still out: the button is off, and this says why.
  const why = startDlg.busy ? '前の依頼を送信中…'
    : full ? `worker の枠が空いていません（稼働 ${state.workerSlots.busy} / 上限 ${state.workerSlots.max}）。どれかが終わってから依頼してください` : '';
  slots.hidden = !why;
  setText(slots, why);
  sessEl('start-submit').disabled = !!problem || full || startDlg.busy || !h || !state.sessionStart?.agent;
}

sessEl('start-instruction').addEventListener('input', () => {
  syncStartDialog();
});
sessEl('start-name').addEventListener('input', () => { startDlg.nameEdited = true; syncStartDialog(); });
sessEl('start-hub').addEventListener('change', syncStartDialog);
sessEl('start-instruction').addEventListener('keydown', e => {
  if (!isConfirmKey(e)) return;
  e.preventDefault();
  submitStart();
});

async function submitStart(e) {
  if (e) e.preventDefault();
  // The hub may take a moment to answer, and a second press would ask it twice.
  if (startDlg.busy || sessEl('start-submit').disabled) return;
  const opening = dialogOpening.start;
  startDlg.busy = true;
  showDlgError('start-error', '');
  syncStartDialog();
  const instruction = sessEl('start-instruction').value.trim();
  const hub = sessEl('start-hub').value;
  const worktreeName = sessEl('start-name').value.trim();
  const before = (state.sessions || []).map(s => s.id);
  const sel = sessView.selectedId;
  const line = `adj send --kind session --from dashboard --subject 'start a session: ${worktreeName}'`;
  try {
    const data = await api('/api/sessions', {
      method: 'POST',
      body: JSON.stringify({ instruction, hub, worktreeName, agent: sessEl('start-agent').value }),
    });
    // A request that got here is in the hub's inbox, whether or not the hub could be started:
    // the dialog is done, and the row in the tree carries what is left to do.
    sessView.starts.push({
      hubId: data.hub || hub, name: data.worktreeName || worktreeName, message: data.message || null,
      at: Date.now(), before, sel, hubStartError: data.hubStartError || null, goneAt: null,
      hubStartedAt: data.hubStarted ? Date.now() : null,
    });
    note(line, !!data.hubStartError, (data.hubStartError ? `hub を起動できませんでした: ${data.hubStartError}` : `${data.worktreeName} の起動を依頼`) + startedOrHanded(data.hubStarted, data.handed));
    if (opening === dialogOpening.start) closeDialogById('start-dialog');
    if (data.hubStartError) showSessNotice(`依頼は受信箱に届きましたが、hub を起動できませんでした: ${data.hubStartError}`, true);
    else showSessNotice(`${data.worktreeName} の起動を hub に依頼しました`);
    await refresh(true);
    renderSessionsView();
  } catch (err) {
    note(`${line} → ${err.message}`, true);
    if (opening === dialogOpening.start) showDlgError('start-error', err.message);
  } finally {
    startDlg.busy = false;
    syncStartDialog();
  }
}
sessEl('start-form').addEventListener('submit', submitStart);

/* ── The rows of sessions asked for and not started yet ── */
const PEND_GIVE_UP_MS = 15000;
const PEND_SLOW_MS = 10 * 60 * 1000;
const escRegex = t => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const pendingNameOf = subject => /^start a session: (\S+)$/.exec(subject || '')?.[1] || null;
const isSessionRequest = m => m.kind === 'session' && m.from === 'dashboard';

function startInInbox(p) {
  const h = (state.hubs || []).find(x => x.id === p.hubId);
  return !!h && (h.inbox || []).some(m => isSessionRequest(m)
    && (p.message ? m.name === p.message : pendingNameOf(m.subject) === p.name));
}

/* The worker the hub made for `p`: its name is the one asked for, or that with `-2` and so on
   when the hub found it taken. */
function startedSession(p, claimed) {
  const re = new RegExp(`^${escRegex(p.name)}(-\\d+)?$`);
  return (state.sessions || []).find(s => s.kind === 'worker' && (hubOfSession(s) || repoHubId()) === p.hubId
    && !p.before.includes(s.id) && !claimed.has(s.id) && workerNames(s).some(n => re.test(n)));
}

/* Called as the tree is drawn, so that what it shows is decided by one read of the state: a
   request that has become a session leaves the list, and one the hub took from its inbox
   without starting anything begins its count to giving up. */
function settleStarts() {
  const now = Date.now();
  const kept = [];
  // A session answers one request: two asking for the same name each get their own.
  const claimed = new Set();
  for (const p of sessView.starts) {
    const found = startedSession(p, claimed);
    if (found) {
      claimed.add(found.id);
      // Its request may still be in the inbox for a moment: hidden, so it is not drawn twice.
      sessView.dismissed.push(`${p.hubId}/${p.message || p.name}`);
      // Only when the person has not moved on to another session since asking.
      if (p.sel === sessView.selectedId) setTimeout(() => { if (view === 'sessions' && sessView.selectedId === p.sel) selectSession(found.id); }, 0);
      continue;
    }
    if (startInInbox(p)) p.goneAt = null;
    else if (p.goneAt == null) p.goneAt = now;
    kept.push(p);
  }
  sessView.starts = kept;
}

function pendingRow(h, name, key, p, at, now) {
  const present = !!h.state?.present;
  const startWhy = h.parent && !h.key ? 'キーが分からないため起動できません。adj hub --hub <キー> で起動してください'
    : !state.hubStart?.available ? NO_START : '';
  const base = { key, hubId: h.id, name, canStart: !startWhy, startWhy, busy: sessBusy.has(`pend-${key}`) };
  if (!present && p?.hubStartError) return { ...base, kind: 'stopped', text: `hub を起動できませんでした: ${p.hubStartError}`, alert: true };
  // Started a moment ago and its record not written yet: it is coming up, not stopped.
  if (!present && p?.hubStartedAt != null && now - p.hubStartedAt < HUB_STARTING_MS) {
    return { ...base, kind: 'stopped', text: 'hub を起動中…', canStart: false, startWhy: 'hub を起動しています' };
  }
  if (!present) return { ...base, kind: 'stopped', text: 'hub が止まっています…' };
  if (at != null && now - at >= PEND_SLOW_MS) return { ...base, kind: 'slow', text: '時間がかかっています' };
  return { ...base, kind: 'waiting', text: '起動を依頼中…' };
}

/* Derived from the hubs' inboxes, so a request made before a reload is still shown, plus the
   requests this page made that the hub has taken and not yet answered with a session. */
function sessionPendingRows() {
  settleStarts();
  const now = Date.now();
  const rows = [];
  for (const h of state.hubs || []) {
    for (const m of h.inbox || []) {
      const name = isSessionRequest(m) ? pendingNameOf(m.subject) : null;
      const key = `${h.id}/${m.name}`;
      if (!name || sessView.dismissed.includes(key)) continue;
      const p = sessView.starts.find(x => x.hubId === h.id && (x.message ? x.message === m.name : x.name === name));
      const stamp = stampSecs(m.at);
      rows.push(pendingRow(h, name, key, p, p?.at ?? (stamp == null ? null : stamp * 1000), now));
    }
  }
  for (const p of sessView.starts) {
    if (startInInbox(p)) continue;
    const key = `${p.hubId}/${p.message || p.name}`;
    const hub = (state.hubs || []).find(x => x.id === p.hubId);
    if (!hub || sessView.dismissed.includes(key)) continue;
    const base = { key, hubId: p.hubId, name: p.name, canStart: false, startWhy: '', busy: false };
    rows.push(now - p.goneAt >= PEND_GIVE_UP_MS
      ? { ...base, kind: 'failed', alert: true, text: 'hub はセッションを起動しませんでした（worker の空きが無いか、起動に失敗）。hub の端末を確認してください' }
      : { ...base, kind: 'waiting', text: '起動を依頼中…' });
  }
  return rows;
}

function pendingRowHtml(p) {
  const icon = p.kind === 'stopped' || p.kind === 'failed' ? 'error' : p.kind === 'slow' ? 'schedule' : STATE_ICON.pending;
  const btn = (act, label, attrs = '') =>
    `<button type="button" class="btn-m3-text" data-pend-act="${act}" data-pend="${esc(p.key)}"${attrs}>${esc(label)}</button>`;
  const buttons = [];
  if (p.kind === 'stopped') buttons.push(btn('start-hub', 'hub を起動', p.canStart && !p.busy ? '' : ` disabled title="${esc(p.startWhy)}"`));
  if (p.kind === 'failed') buttons.push(btn('select-hub', 'hub を選ぶ'), btn('dismiss', '閉じる'));
  if (p.kind === 'slow') buttons.push(btn('dismiss', '閉じる'));
  const cls = p.kind === 'waiting' ? '' : p.kind;
  return `<div class="sess-row pending ${cls}" data-pend-row role="${p.alert ? 'alert' : 'status'}" title="${esc(`${p.name}\n${p.text}`)}">
    <span class="material-symbols-outlined sess-ico" aria-hidden="true">${icon}</span>
    <span class="sess-row-text"><span class="sess-row-key">${esc(p.name)}</span><span class="sess-row-sub">${esc(p.text)}</span></span>` +
    (buttons.length ? `<span class="sess-pend-btns">${buttons.join('')}</span>` : '') + '</div>';
}

sessEl('sess-tree').addEventListener('click', e => {
  const b = e.target.closest('[data-pend-act]');
  // In the icon-only rail a row has no words or buttons: pressing it opens the rail to them.
  if (!b && e.target.closest('[data-pend-row]') && railCollapsed()) {
    prefs.sessionsRail = 'open';
    savePrefs();
    return renderSessionsView();
  }
  if (!b || b.disabled) return;
  const key = b.dataset.pend;
  const row = sessionPendingRows().find(r => r.key === key);
  if (!row) return;
  if (b.dataset.pendAct === 'select-hub') return selectSession(row.hubId);
  if (b.dataset.pendAct === 'dismiss') {
    sessView.dismissed.push(key);
    sessView.starts = sessView.starts.filter(p => `${p.hubId}/${p.message || p.name}` !== key);
    return renderSessionsView();
  }
  return retryHubStart(row);
});

/* Starts the hub of a request that is waiting in a stopped one's inbox. A failure stays on the
   row, since the request is still queued and the row is where it is waiting. */
function retryHubStart(row) {
  const h = (state.hubs || []).find(x => x.id === row.hubId);
  if (!h) return;
  const tracked = () => {
    let p = sessView.starts.find(x => x.hubId === row.hubId && x.name === row.name);
    if (!p) {
      p = { hubId: row.hubId, name: row.name, message: row.key.slice(row.hubId.length + 1), at: null, before: (state.sessions || []).map(s => s.id), sel: sessView.selectedId, hubStartError: null, goneAt: null, hubStartedAt: null };
      sessView.starts.push(p);
    }
    return p;
  };
  const run = sessAct(`pend-${row.key}`, `hub を起動 (${hubShortName(h)})`, async () => {
    renderSessionsView();
    try {
      const data = await api(`/api/hubs/${enc(h.id)}/start`, { method: 'POST', body: '{}' });
      tracked().hubStartError = null;
      if (!data.alreadyRunning) tracked().hubStartedAt = Date.now();
      const text = data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました';
      note(h.key ? `adj hub --tab --hub=${h.key}` : 'adj hub --tab', false, text);
      showSessNotice(text);
      await refresh(true);
    } catch (err) {
      tracked().hubStartError = err.message;
      throw err;
    }
  });
  // Drawn once more when the press is over: the row's buttons were dimmed while it ran.
  return Promise.resolve(run).then(() => renderSessionsView());
}

// The give-up and the hub's answer are times, not state changes: nothing else would redraw the
// tree for them while the page sits still.
setInterval(() => {
  if (view !== 'sessions') return;
  if (sessView.starts.length) renderSessionTree();
  syncStartDialog();
}, 2000);

/* ── Giving a session with no task a task ── */
const linkDlg = { s: null, data: {}, errors: {}, busy: false };

/* Every board of this page's server that can hold a task, with the hub it belongs to. Without
   the resident server there is only this page's own. */
function linkBoards() {
  const seen = new Map();
  for (const h of state.hubs || []) {
    if (h.parent && !h.key) continue;
    const base = state.resident && h.slug ? `/b/${h.slug}` : BASE;
    if (!seen.has(base)) seen.set(base, { base, hub: h, own: base === BASE });
  }
  return [...seen.values()];
}

const linkData = b => b.own ? state : linkDlg.data[b.base] || null;
const linkMode = () => document.querySelector('input[name=link-mode]:checked').value;

/* What an existing task has to be to take a session: not finished, not Jules', not a
   postscript, and with nobody working on it. */
function linkCandidates() {
  const out = [];
  linkBoards().forEach((b, i) => {
    const data = linkData(b);
    for (const t of data?.tasks || []) {
      if (t.status === 'done' || t.status === 'cancelled' || t.executor === 'jules' || t.kind === 'tell-worker') continue;
      if (workerOf(t, data)?.present) continue;
      out.push({ value: `${i}:${t.id}`, board: b, task: t });
    }
  });
  return out;
}

function openLinkDialog(s, mode) {
  dialogOpening.link++;
  linkDlg.s = s;
  linkDlg.data = {};
  linkDlg.errors = {};
  sessEl('link-session').textContent = `${sessionKey(s)}${s.branch ? `（${s.branch}）` : ''}`;
  for (const r of document.querySelectorAll('input[name=link-mode]')) r.checked = r.value === mode;
  for (const id of ['link-title-input', 'link-body']) sessEl(id).value = '';
  sessEl('link-done-when').value = 'pr';
  sessEl('link-file-issue').checked = false;
  sessEl('link-phase').innerHTML = Object.entries(PHASE_LABEL).map(([k, v]) => `<option value="${esc(k)}">${esc(v)}</option>`).join('');
  sessEl('link-phase').value = PHASE_LABEL[s.phase] ? s.phase : 'implement';
  sessEl('link-task').innerHTML = '';
  showDlgError('link-error', '');
  syncLinkDialog();
  sessEl('link-dialog').showModal();
  // The other hubs' boards are read now; this page's own is already in `state`.
  for (const b of linkBoards().filter(x => !x.own)) {
    boardApi(b.base, '/api/state').then(data => { linkDlg.data[b.base] = data; }).catch(e => {
      linkDlg.errors[b.base] = `${hubShortName(b.hub)}: ${e.message}`;
    }).then(() => {
      if (linkDlg.s === s && sessEl('link-dialog').open) { renderLinkTasks(); syncLinkDialog(); }
    });
  }
  renderLinkTasks();
  setTimeout(() => sessEl(mode === 'new' ? 'link-title-input' : 'link-task').focus(), 50);
}

function renderLinkTasks() {
  const sel = sessEl('link-task');
  const keep = sel.value;
  const cands = linkCandidates();
  const many = linkBoards().length > 1;
  sel.innerHTML = cands.map(c => {
    const where = many ? `${hubShortName(c.board.hub)} ・ ` : '';
    return `<option value="${esc(c.value)}">${esc(`${c.task.title || c.task.id}（${where}${c.task.status}）`)}</option>`;
  }).join('');
  if (cands.some(c => c.value === keep)) sel.value = keep;
  else if (cands.length) sel.selectedIndex = 0;
  const loading = linkBoards().some(b => !b.own && !linkData(b) && !linkDlg.errors[b.base]);
  const errors = Object.values(linkDlg.errors);
  sessEl('link-task-note').textContent = errors.length ? `読めなかったボードがあります（${errors.join('、')}）`
    : loading ? 'ほかの hub のボードを読み込んでいます…'
      : cands.length ? '' : '紐づけられるタスクがありません（完了・取り消し済み、worker が動いている、Jules 向け、の各タスクは出ません）';
}

/* The hub the session belongs to now, as `hubs[]` lists it. */
const linkCurrentHub = () => (state.hubs || []).find(h => h.id === (hubOfSession(linkDlg.s) || repoHubId())) || null;

/* The hub the link puts the session under: a new task is made where the session already is,
   an existing one is on the board of the hub it belongs to. */
function linkTarget() {
  if (linkMode() === 'new') return linkCurrentHub();
  const cand = linkCandidates().find(c => c.value === sessEl('link-task').value);
  return cand ? cand.board.hub : null;
}

/* Says before the person presses the button where the session ends up, since a task on
   another hub's board takes the session with it. */
function linkMoveNote() {
  const cur = linkCurrentHub();
  const target = linkTarget();
  if (!target) return '';
  const short = h => hubShortName(h);
  const parts = [];
  const stopped = !target.state?.present;
  if (linkMode() === 'new') {
    if (target.parent) parts.push(`親タスク ${short(target)} の子タスクとして作ります`);
    if (sessEl('link-file-issue').checked && stopped) {
      parts.push(state.resident && state.hubStart?.available
        ? 'hub が止まっているため、起票の依頼と同時に起動します'
        : 'hub が止まっています。起動するまで Issue は起票されません');
    }
  } else if (!cur || target.id !== cur.id) {
    parts.push(target.parent ? `このセッションは親タスク ${short(target)} の hub に移ります`
      : cur?.parent ? `このセッションは親タスク ${short(cur)} の hub を離れ、リポジトリの hub に移ります`
        : 'このセッションはリポジトリの hub に移ります');
    if (stopped) parts.push('hub は止まっています。起動するまで、このセッションの gate などへの返事は処理されません');
  }
  return parts.join('。');
}

function syncLinkDialog() {
  if (!sessEl('link-dialog').open) return;
  const mode = linkMode();
  sessEl('link-new').hidden = mode !== 'new';
  sessEl('link-existing').hidden = mode !== 'existing';
  sessEl('link-title').textContent = mode === 'new' ? 'セッションをタスクにする' : 'セッションを既存のタスクに紐づける';
  const text = linkMoveNote();
  const noteEl = sessEl('link-move-note');
  noteEl.hidden = !text;
  setText(noteEl, text);
  sessEl('link-submit').disabled = linkDlg.busy;
}

for (const r of document.querySelectorAll('input[name=link-mode]')) r.addEventListener('change', syncLinkDialog);
sessEl('link-task').addEventListener('change', syncLinkDialog);
sessEl('link-file-issue').addEventListener('change', syncLinkDialog);
sessEl('link-body').addEventListener('keydown', e => {
  if (!isConfirmKey(e)) return;
  e.preventDefault();
  submitLink();
});

async function submitLink(e) {
  if (e) e.preventDefault();
  const s = linkDlg.s;
  if (!s || linkDlg.busy) return;
  const mode = linkMode();
  const target = linkTarget();
  const phase = sessEl('link-phase').value;
  let body;
  let line;
  if (mode === 'new') {
    const title = sessEl('link-title-input').value.trim();
    const text = sessEl('link-body').value.trim();
    if (!title && !text) return showDlgError('link-error', 'タイトルか内容のどちらかを入力してください');
    const kind = sessEl('link-file-issue').checked ? 'file-and-start' : 'start';
    body = { newTask: { title, body: text, doneWhen: sessEl('link-done-when').value, kind }, hub: target?.id, phase };
    line = `adj task add --kind ${kind} --worktree ${s.worktree}`;
  } else {
    const cand = linkCandidates().find(c => c.value === sessEl('link-task').value);
    if (!cand) return showDlgError('link-error', '紐づけるタスクを選んでください');
    body = { task: cand.task.id, hub: cand.board.hub.id, phase };
    line = `adj task update --id ${cand.task.id} --worktree ${s.worktree}`;
  }
  const opening = dialogOpening.link;
  const from = boardOfSession(s).base;
  const to = target ? (state.resident && target.slug ? `/b/${target.slug}` : BASE) : from;
  linkDlg.busy = true;
  showDlgError('link-error', '');
  syncLinkDialog();
  try {
    const data = await api(`/api/sessions/${enc(s.id)}/link`, { method: 'POST', body: JSON.stringify(body) });
    const id = data.task?.id || '';
    let text = `${id} に紐づけました`;
    let bad = false;
    if (data.fileIssueError) {
      bad = true;
      text += `。ただし Issue の起票を hub に頼めませんでした: ${data.fileIssueError}`;
    } else if (data.fileIssue) {
      text += '。hub に Issue の起票を依頼しました';
    }
    if (data.hubStartError) {
      bad = true;
      text += `。hub を起動できませんでした: ${data.hubStartError}`;
    }
    note(line, bad, text + (data.fileIssue ? startedOrHanded(data.hubStarted, data.fileIssue.handed) : ''));
    if (opening === dialogOpening.link) closeDialogById('link-dialog');
    showSessNotice(text, bad);
    await refresh(true);
    refetchSideBoard(from);
    refetchSideBoard(to);
  } catch (err) {
    note(`${line} → ${err.message}`, true);
    if (opening === dialogOpening.link) showDlgError('link-error', err.message);
  } finally {
    linkDlg.busy = false;
    syncLinkDialog();
  }
}
sessEl('link-form').addEventListener('submit', submitLink);
sessEl('link-dialog').addEventListener('close', () => { linkDlg.s = null; });
