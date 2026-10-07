/* ── The task panel ────────────────────────────────────────────────────────────────────────
   One task at a time, beside the sidebar: `selectedTaskId` is the task it shows and `nav.pane`
   the tab. Where it sits is only a class on body (panel-right, panel-pop), so the terminal in
   #tp-term-host is never moved, rebuilt or redrawn: a terminal that is moved reconnects. The
   redraws below touch the parts around it, and never the host or what holds it. */
const tp = id => document.getElementById(id);
const taskById = id => (state.tasks || []).find(t => t.id === id);

function markSelectedCards() {
  for (const card of document.querySelectorAll('#boards .card')) {
    card.classList.toggle('selected', card.dataset.id === selectedTaskId);
  }
}

/* The panel on `id`, as the address says: no history entry is made, the address is what asked. */
function showTaskPanel(id) {
  pendingTask = null;
  if (selectedTaskId !== id) sessView.git = null;
  selectedTaskId = id;
  markSelectedCards();
  renderTaskPanel();
}

/* A click on a card or one of its buttons: another card is a step in the history, the other tab
   of the card that is open replaces the one it is on. */
function openTaskPanel(id, pane = 'detail') {
  pendingTask = null;
  if (selectedTaskId !== id) sessView.git = null;
  selectedTaskId = id;
  markSelectedCards();
  go({ task: id, pane }, { replace: id === nav.task });
}

/* The panel's state, without the address: for a move the address already made. */
function hideTaskPanelState() {
  pendingTask = null;
  selectedTaskId = null;
  panelScrolledFor = null;
  // Opened again, a session's git state is read again.
  sessView.git = null;
  disposePanelTerminal();
  markSelectedCards();
  renderTaskPanel();
}

/* A screen with no panel (the review queue): the address follows it. */
function dismissTaskPanel() {
  hideTaskPanelState();
  if (nav.task) setNav({ task: null, pane: 'detail' });
}

function closeTaskPanel() {
  hideTaskPanelState();
  if (nav.task) go({ task: null, pane: 'detail' });
}

/* Where panels open is a remembered mode: 'pop' is the dialog, 'left' and 'right' the sidebar on that side.
   Closing never changes it; only choosing one of these does. */
function placePanel(where) {
  if (where === 'pop') prefs.panelDialog = true;
  else {
    prefs.panelDialog = false;
    prefs.panelDock = where;
  }
  savePrefs();
  renderTaskPanel();
}

/* A session is there to open a terminal on when its worktree has one at all. */
const hasSession = s => !!s && sessionState(s) !== 'none';
/* The tab shown: ターミナル only where there is a session, whatever the address says. */
const paneOf = task => nav.pane === 'term' && hasSession(sessionOfTask(task)) ? 'term' : 'detail';
/* The same for a session with no task, which is its own subject. */
const sessPaneOf = s => nav.pane === 'term' && hasSession(s) ? 'term' : 'detail';

/* The sidebar is the icon rail when the window is narrow, and while the panel sits on its left. */
const narrowRail = matchMedia('(max-width: 1024px)');
function applyRailMode() {
  const cls = document.body.classList;
  const byPanel = cls.contains('panel-open') && !cls.contains('panel-right') && !cls.contains('panel-pop');
  cls.toggle('rail-icons', narrowRail.matches || byPanel);
}
narrowRail.addEventListener('change', applyRailMode);
// Before the first poll has drawn anything, a narrow window already has its icon rail.
applyRailMode();

/* What each part was last drawn from: a part is drawn again only when it changed. */
let panelScrolledFor = null;
const panelSig = { head: '', tabs: '', gate: '', rest: '', bar: '', ph: '' };
function setPanelPart(part, el, html) {
  if (panelSig[part] === html) return;
  panelSig[part] = html;
  el.innerHTML = html;
}

function renderTaskPanel() {
  const panel = tp('task-panel');
  if (!panel) return;
  // A session that has been linked to a task since is shown as that task, and the address follows;
  // its terminal is kept (syncTermSlot).
  if (isSessRef(selectedTaskId)) {
    const ref = panelRefOf(selectedTaskId);
    if (ref !== selectedTaskId) {
      if (nav.task === selectedTaskId) setNav({ task: ref });
      selectedTaskId = ref;
      markSelectedCards();
    }
  }
  const hub = hubOfRef(selectedTaskId);
  const task = selectedTaskId && !isHubRef(selectedTaskId) ? taskById(selectedTaskId) : null;
  const sess = !hub && !task ? sessOfRef(selectedTaskId) : null;
  // A card the board no longer lists, a hub that left its list, or a session that is gone takes
  // the panel with it.
  if (selectedTaskId && !task && !hub && !sess) return dismissTaskPanel();
  const shown = !!(task || hub || sess) && (view === 'board' || view === 'sessions');
  const cls = document.body.classList;
  panel.hidden = !shown;
  tp('tp-scrim').hidden = !(shown && prefs.panelDialog);
  cls.toggle('panel-open', shown);
  cls.toggle('panel-right', prefs.panelDock === 'right');
  cls.toggle('panel-pop', shown && prefs.panelDialog);
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
  applyRailMode();
  // The list marks the session the panel is open on.
  if (view === 'sessions') applySessionSelection();
  markHubButtons();
  // Closed, not just out of view: what was typed for the task goes with it.
  if (!task && !hub && !sess) return renderHandForm(null);
  // In the task view the panel waits, with its terminal, for the board to come back.
  if (!shown) return;

  const colId = task ? columnOf(task) : null;
  const gate = task ? openGate(task) : null;
  const s = hub ? hubSessionOf(hub) : sess || sessionOfTask(task);
  const pane = hub ? hubPaneOf(hub, s) : sess ? sessPaneOf(s) : paneOf(task);

  setPanelPart('head', tp('tp-head'), hub ? hubPanelHeadHtml(hub, s) : sess ? sessPanelHeadHtml(s) : panelHeadHtml(task));
  setPanelPart('tabs', tp('tp-tabs'), hub ? hubPanelTabsHtml(hub, s, pane) : sess ? panelTabsHtml(null, s.waiting, s, pane) : panelTabsHtml(task, gate, s, pane));

  // Shown before the terminal is mounted: a hidden host has no size to fit to.
  const reveal = pane === 'term' && tp('tp-term').hidden;
  tp('tp-detail').hidden = pane === 'term';
  tp('tp-term').hidden = pane !== 'term';
  // Another task starts at the top, not where the last one was scrolled to: set once the pane
  // is shown, since a hidden one has no scroll to set.
  if (pane === 'detail' && panelScrolledFor !== selectedTaskId) {
    panelScrolledFor = selectedTaskId;
    tp('tp-detail').scrollTop = 0;
  }
  syncPanelTerminal(selectedTaskId, s, pane);
  setPanelPart('bar', tp('tp-term-bar'), termBarHtml(s, panelTerm, true, task));
  const ph = tp('tp-term-ph');
  ph.hidden = !!panelTerm.term;
  setPanelPart('ph', ph, panelTerm.term ? '' : termPlaceholderHtml(s));
  if (reveal && panelTerm.term) {
    // It was sized while hidden, which it skips; asked again now that it has a size.
    panelTerm.term.fit();
    panelTerm.term.focus();
  }

  setPanelPart('links', tp('tp-links'), hub || sess ? '' : ghRowsHtml(task));
  setPanelPart('gate', tp('tp-gate'), hub || sess ? sessGateHtml(s) : panelGateHtml(task, gate));
  setPanelPart('rest', tp('tp-rest'), hub ? hubDetailHtml(hub, s) : sess ? sessDetailHtml(s, pane) : panelRestHtml(task, colId));
  renderHandForm(!hub && !sess && colId === 'backlog' ? task : null);
}

function panelHeadHtml(task) {
  const hcol = humanColOf(task);
  const stuck = stuckOf(task);
  const colObj = COLUMNS.find(c => c.id === columnOf(task));
  const pill = hcol ? `<span class="m3-pill pill-warn">${esc(humanLabel(hcol))}を待っています</span>`
    : stuck ? `<span class="m3-pill pill-err">${esc(stuck)}</span>`
    : `<span class="m3-pill pill-blue">${esc(colObj ? colObj.label : task.status)}</span>`;
  const b = selectedBoard();
  const origin = b ? boardName(b) : (state.repo || '').split('/').pop();
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(task.id)}">${esc(task.id)}</span>
        ${origin ? `<span class="origin-chip" title="${esc(b ? `${boardName(b)} (${b.nwo})` : state.repo || '')}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        ${pill}
      </div>
      <h2 class="tp-title">${esc(task.title)}${titlePendingPill(task)}</h2>
    </div>
    ${panelBtnsHtml('<button type="button" class="btn-m3-text tp-jump" data-tp-jump title="エージェントのボードでこのカードを見る">カードへ</button>')}`;
}

/* The head's buttons, which are the same for a task and a hub: `jump` is the one of its own. */
function panelBtnsHtml(jump) {
  const place = (where, icon, label, on) =>
    `<button type="button" class="tool-btn${on ? ' on' : ''}" data-tp-place="${where}" title="${label}" aria-label="${label}" aria-pressed="${on}"><span class="material-symbols-outlined" aria-hidden="true">${icon}</span></button>`;
  return `<div class="tp-head-btns">
      ${jump}
      ${place('left', 'left_panel_open', '左のサイドバーに置く', prefs.panelDock === 'left' && !prefs.panelDialog)}
      ${place('right', 'right_panel_open', '右のサイドバーに置く', prefs.panelDock === 'right' && !prefs.panelDialog)}
      ${place('pop', 'open_in_new', 'ダイアログで開く', prefs.panelDialog)}
      <button type="button" class="tool-btn" data-tp-close title="閉じる" aria-label="閉じる"><span class="material-symbols-outlined" aria-hidden="true">close</span></button>
    </div>`;
}

/* `off` is why the tab cannot be used (its tooltip), or falsy. */
const panelTab = (pane, id, label, extra, off) =>
  `<button type="button" role="tab" id="tp-tab-${id}" class="tp-tab${pane === id ? ' on' : ''}" data-pane="${id}" aria-selected="${pane === id}" aria-controls="${id === 'term' ? 'tp-term' : 'tp-detail'}"${off ? ` disabled title="${esc(off)}"` : ''}>${label}${extra}</button>`;

function panelTabsHtml(task, gate, s, pane) {
  const usable = hasSession(s) && !!state.boardTerminal?.available;
  const hint = hasSession(s) ? '端末はボードから開けません' : 'セッションなし';
  return panelTab(pane, 'detail', '詳細', gate ? '<span class="tp-dot" title="あなたの判断待ちがあります"></span>' : '', '')
    + panelTab(pane, 'term', 'ターミナル', !usable ? `<span class="tp-tab-hint">${hint}</span>` : s?.waiting ? '<span class="tp-wait">入力待ち</span>'
      : s && sessionState(s) === 'permission' ? `<span class="tp-wait">${permissionLabel(s)}</span>` : '', !usable && hint);
}

/* ── 詳細 ── */
const secTitle = text => `<div class="tp-sec-title">${text}</div>`;
const kv = (label, value) => `<div class="tp-kv"><span>${label}</span><strong>${value}</strong></div>`;
const monoKv = (label, value, title = '') => `<div class="tp-kv"><span>${label}</span><code title="${esc(title)}">${esc(value)}</code></div>`;

/* The open gate. A decision that needs no comment is one click; reading the plan or the diff,
   and anything that wants a comment, is the judging screen. Not humanActions(): its reply box
   is found by `textarea[data-reply]`, which two on one page would share. */
function panelGateHtml(task, gate) {
  if (!gate) return '';
  const [label] = kindOf(gate.kind);
  const col = gate.humanCol;
  const why = gate.problem || gate.why || '';
  const reasons = stopWhy(gate);
  const quick = col === 'dispatch' ? [['start', 'play_arrow']]
    : col === 'plan' || col === 'diff' || col === 'verify' ? [['approve', 'check']]
    : [];
  const btn = ([action, icon]) =>
    `<button type="button" class="btn-m3-primary" data-tp-act="${action}"><span class="material-symbols-outlined" style="font-size:16px;">${icon}</span><span>${esc(actLabel(action, gate.kind))}</span></button>`;
  return `
    <div class="m3-card-attention-box">
      <div class="tp-gate-head">
        <span class="material-symbols-outlined" style="font-size:16px;">pending_actions</span>
        <span>【${esc(label)}】あなたの判断待ち</span>
        <span class="tp-gate-wait">${esc(minutesLabel(waitingMinutes(task)))}待ち</span>
      </div>
      <div style="font-size:12.5px;">${esc(gate.title)}</div>
      ${why ? `<div style="font-size:11.5px;white-space:pre-wrap;overflow-wrap:anywhere;">${esc(why)}</div>` : ''}
      ${reasons.length ? `<div style="font-size:11.5px;">止めた理由: ${esc(reasons.join(' / '))}</div>` : ''}
      <div class="tp-gate-actions">
        ${quick.map(btn).join('')}
        <button type="button" class="${quick.length ? 'btn-m3-tonal' : 'btn-m3-primary'}" data-judge="${esc(gate.id)}"><span class="material-symbols-outlined" style="font-size:16px;">arrow_forward</span><span>判定画面を開く</span></button>
        ${col === 'question' && readySessionOfTask(task) ? `<button type="button" class="btn-m3-tonal" data-pane="term"><span class="material-symbols-outlined" style="font-size:16px;">terminal</span><span>ターミナルで答える</span></button>` : ''}
      </div>
    </div>`;
}

const STEPS = [['plan', '計画'], ['implement', '実装'], ['selfreview', 'セルフレビュー'], ['pr', 'PR']];

function panelRestHtml(task, colId) {
  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;
  const at = agentColOf(task);
  // Before the first step nothing is lit; after the last, all of them are.
  const now = at === 'done' ? STEPS.length : STEPS.findIndex(([id]) => id === at);
  const mins = worker?.phase ? phaseMinutes(worker) : null;
  let h = '';

  h += `<div class="m3-filled-card">${secTitle('工程')}
    <ol class="tp-steps">${STEPS.map(([, text], i) => `<li class="${i < now ? 'done' : i === now ? 'now' : ''}">${esc(text)}</li>`).join('')}</ol>
    ${worker?.phase ? `<div class="tp-line">worker は${esc(PHASE_LABEL[worker.phase] || worker.phase)}${worker.present ? '' : '（停止）'}${mins != null ? `（${esc(agoLabel(mins))}から）` : ''}</div>` : ''}
    ${live ? agentFactsHtml(sessionOfTask(task)) : ''}
    <div class="tp-kvs">
      ${kv('完了条件', esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—'))}
      ${kv('止める所', esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt || '—'))}
      ${task.executor === 'jules' ? kv('実装', httpUrl(task.jules?.url)
        ? `<a href="${esc(task.jules.url)}" target="_blank" rel="noopener noreferrer" class="tp-link"><span>Jules ${esc(julesText(task.jules))}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>`
        : `Jules${task.julesSession ? '' : '（計画の承認後に渡す）'}`) : ''}
    </div>
  </div>`;

  // Newest first: the one the worker left last is the one that describes where it is now.
  const records = recordsOf(task).reverse();
  h += `<div class="m3-filled-card">${secTitle('記録（止めずに進んだもの）')}
    ${records.length ? records.map(r => {
      const [label] = kindOf(r.kind);
      const [text, tone] = recordSummary(r);
      const pillClass = tone === 'good' ? 'pill-good' : tone === 'bad' ? 'pill-err' : 'pill-warn';
      const icon = tone === 'good' ? 'check_circle' : tone === 'bad' ? 'cancel' : 'info';
      return `<div class="tp-record">
        <span class="m3-pill ${pillClass}"><span class="material-symbols-outlined" style="font-size:12px;margin-right:2px;">${icon}</span><span>${esc(label)}: ${esc(text)}</span></span>
        <div class="tp-muted">${ago(r.openedAt)}に記録</div>
        <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;" data-record="${esc(r.id)}">全体を見る・差し戻す →</button>
      </div>`;
    }).join('') : '<div class="tp-muted">記録はまだありません</div>'}
  </div>`;
  if (task.julesSession && httpUrl(task.pr) && live) h += relayHtml(task);

  h += `<div class="m3-filled-card">${secTitle('作業場所')}
    ${task.worktree || task.branch ? `<div class="tp-kvs">
      ${monoKv('worktree', baseName(task.worktree) || '—', task.worktree || '')}
      ${monoKv('ブランチ', task.branch || '—')}
    </div>` : '<div class="tp-muted">worktree はまだありません</div>'}
    ${task.worktree ? `<button type="button" class="btn-m3-tonal tp-ide" title="${ideTitle()}" data-ide="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:16px;">code</span><span>IDE</span></button>` : ''}
  </div>`;

  // The latest few only: the whole history is a click away in the task view's 経過 tab.
  const all = gatesOf(task);
  h += `<div class="m3-filled-card" style="display:flex;flex-direction:column;">${secTitle('経過')}
    ${timelineHtml(task, all, 5)}
    <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;margin-top:6px;" data-history="${esc(task.id)}">経過をすべて見る →</button>
  </div>`;
  if (task.instruction && colId !== 'backlog') {
    h += `<div class="m3-filled-card">${secTitle('エージェントへの申し送り（指示）')}<p class="tp-text">${esc(task.instruction)}</p></div>`;
  }
  if (task.body) {
    h += `<div class="m3-filled-card">${secTitle('依頼内容・プロンプト')}<p class="tp-text">${esc(task.body)}</p></div>`;
  }
  return h;
}

/* ── A hub in the panel ──────────────────────────────────────────────────────────────────────
   `selectedTaskId` is `hub:<id>`, and the panel draws the same parts: the hub's session is the
   terminal, and 詳細 is what the hub is handling and what can be done to it. */
/* The page's own hub is the one its state counts; another board's hub is counted by its row in
   the board list. */
const hubOther = h => { const own = pageHub(); return !!own && own.id !== h.id; };
const hubRowOf = h => (multiBoard ? boards.find(b => b.slug === h.slug) : null) || null;
/* Where the hub's own requests go: its board's route when it is not the page's. */
const hubBaseOf = h => multiBoard && hubOther(h) ? `/b/${h.slug}` : BASE;

/* Why the hub's terminal cannot be opened, as [the tab's hint, its tooltip]; null when it can. */
function hubTermWhy(h, s) {
  if (!s.present) return hubStartingNow(h) ? ['起動中', 'hub を起動しています'] : ['hub 停止中', 'hub が止まっています'];
  if (!state.boardTerminal?.available) return ['端末はボードから開けません', '端末はボードから開けません'];
  if (!boardTerminalReady(s)) return ['tmux の外', 'tmux の外で動いている hub は、ボードから端末を開けません'];
  return null;
}
const hubPaneOf = (h, s) => nav.pane === 'term' && !hubTermWhy(h, s) ? 'term' : 'detail';

function hubPanelHeadHtml(h, s) {
  const row = hubRowOf(h);
  const since = sinceLabel(row?.hubLastAlive);
  const [pillText, pillCls] = hubRestartPending(h) ? ['再起動しています…', 'pill-neutral']
    : hubStartingNow(h) ? ['起動しています…', 'pill-neutral']
    : !s.present ? [`停止中${since ? ` · ${since}` : ''}`, 'pill-err']
    : s.waiting ? ['入力待ち', 'pill-warn']
    : sessionState(s) === 'permission' ? [permissionLabel(s), 'pill-warn']
    : ['稼働中', 'pill-good'];
  const origin = row ? boardName(row) : repoName();
  const nwo = row?.nwo || state.repo || '';
  const title = h.parent ? `親タスク hub — ${[h.key, h.title].filter(Boolean).join(' ') || '（キー不明）'}`
    : `リポジトリ hub — ${nwo}`;
  const jump = multiBoard && hubOther(h)
    ? '<button type="button" class="btn-m3-text tp-jump" data-tp-board title="この hub のボードを開く">ボードへ</button>' : '';
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(h.id)}">hub</span>
        ${origin ? `<span class="origin-chip" title="${esc(nwo)}"><span class="material-symbols-outlined" aria-hidden="true">${h.parent ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        <span class="m3-pill ${pillCls}">${esc(pillText)}</span>
      </div>
      <h2 class="tp-title">${esc(title)}</h2>
    </div>
    ${panelBtnsHtml(jump)}`;
}

function hubPanelTabsHtml(h, s, pane) {
  const why = hubTermWhy(h, s);
  return panelTab(pane, 'detail', '詳細', '', '')
    + panelTab(pane, 'term', 'ターミナル', why ? `<span class="tp-tab-hint">${esc(why[0])}</span>` : s.waiting ? '<span class="tp-wait">入力待ち</span>'
      : sessionState(s) === 'permission' ? `<span class="tp-wait">${permissionLabel(s)}</span>` : '', why && why[1]);
}

const HUB_LIST_MAX = 5;
const hubListMore = n => n > 0 ? `<div class="tp-muted">ほか ${n} 件</div>` : '';

function hubDetailHtml(h, s) {
  const other = hubOther(h);
  const row = hubRowOf(h);
  const startWhy = hubStartWhy(h) || hubWaitWhy(h);
  let html = '';
  if (!s.present) {
    const since = sinceLabel(row?.hubLastAlive);
    html += `<div class="m3-card-attention-box">
      <div class="tp-gate-head"><span class="material-symbols-outlined" style="font-size:16px;">stop_circle</span><span>hub は${since ? `${esc(since)}から` : ''}止まっています。起動するとここでターミナルを開けます。</span></div>
      <div class="tp-gate-actions"><button type="button" class="btn-m3-primary" data-tp-hub="start"${startWhy ? ' disabled' : ''} title="${esc(startWhy || 'tmux の新しいウィンドウで adj hub を実行します')}"><span class="material-symbols-outlined" style="font-size:16px;">play_arrow</span><span>hub を起動</span></button></div>
    </div>`;
  }

  // The page's own board is read from its state; another board's from its row in the list.
  const line = other ? [] : (state.tasks || []).filter(t => t.status === 'queued').sort((a, b) => (a.order || 0) - (b.order || 0));
  const queued = other ? row?.queued : line.length;
  const counts = [];
  if (!other && state.workerSlots) counts.push(kv('worker', `${state.workerSlots.busy} / ${state.workerSlots.max} 稼働`));
  else if (other && row) counts.push(kv('作業中', `${row.working || 0} 件`));
  if (!other) counts.push(kv('あなたの確認待ち', `${waitingIn()} 件`));
  else if (row) counts.push(kv('あなたの確認待ち', `${row.waiting || 0} 件`));
  if (queued != null) counts.push(kv('待ちキュー', `${queued} 件`));
  counts.push(kv('受信箱', `${h.inboxCount || 0} 件`));
  html += `<div class="m3-filled-card">${secTitle('いまの状態')}<div class="tp-kvs">${counts.join('')}</div>${agentFactsHtml(s)}</div>`;

  // Only the page's own board has its tasks to name; another board's are read on its own page.
  if (!other) {
    html += `<div class="m3-filled-card">${secTitle('待ちキュー')}${line.length
      ? `<ul class="sess-side-list">${line.slice(0, HUB_LIST_MAX).map(t => `<li><button type="button" class="linkish" data-tp-task="${esc(t.id)}">${esc(t.title)}</button><span class="who">${esc(t.id)}</span></li>`).join('')}</ul>${hubListMore(line.length - HUB_LIST_MAX)}`
      : '<div class="tp-muted">待ちはありません</div>'}</div>`;
  }

  // The hub reports the newest first and `inboxCount` is the whole of it. What is still to be read
  // comes first, oldest first, so the message the board's entry names the age of is in view.
  const unread = m => m.counted && !m.seen;
  const listed = h.inbox || [];
  const inbox = [...listed.filter(unread).reverse(), ...listed.filter(m => !unread(m))].slice(0, HUB_LIST_MAX);
  html += `<div class="m3-filled-card">${secTitle('受信箱')}${inbox.length
    ? `<ul class="sess-side-list">${inbox.map(m => `<li><div>${unread(m) ? '<span class="m3-pill pill-warn">未確認</span> ' : ''}${esc(m.subject || m.name)}</div><div class="who">${esc([m.kind, m.from, m.at ? when(m.at) : ''].filter(Boolean).join(' ・ '))}</div></li>`).join('')}</ul>${hubListMore((h.inboxCount || 0) - inbox.length)}`
    : '<div class="tp-muted">受信箱は空です</div>'}</div>`;

  // A stopped hub's start is the banner's.
  const act = hubActionOf(s);
  const own = act && act.act !== 'hub-start'
    ? `<button type="button" class="btn-m3-tonal" data-tp-hub="${act.act.slice(4)}"${act.disabled ? ' disabled' : ''} title="${esc(act.title)}"><span class="material-symbols-outlined" style="font-size:16px;">${act.icon}</span><span>${esc(act.label)}</span></button>` : '';
  const restartBtn = canRestart(s) ? (() => {
    const why = restartWhy(s);
    return `<button type="button" class="btn-m3-tonal" data-tp-hub="restart"${why ? ' disabled' : ''} title="${esc(why || '今の会話のまま、止めて起動し直します（adj hub --resume）')}"><span class="material-symbols-outlined" style="font-size:16px;">autorenew</span><span>セッションを再起動…</span></button>`;
  })() : '';
  html += `<div class="m3-filled-card">${secTitle('操作')}
    <div class="tp-gate-actions">
      <button type="button" class="btn-m3-tonal" data-tp-hub="next" title="adj send --kind next (着手を促す)"><span class="material-symbols-outlined" style="font-size:16px;">bolt</span><span>着手を促す</span></button>
      <button type="button" class="btn-m3-tonal" data-tp-hub="sync" title="再同期 (adj refresh)"><span class="material-symbols-outlined" style="font-size:16px;">refresh</span><span>再同期</span></button>
      ${own}
      ${restartBtn}
    </div>
    <button type="button" class="btn-m3-text tp-reset" data-tp-hub="reset"${startWhy ? ' disabled' : ''} title="${esc(startWhy || 'hub をリセット：新しい会話で hub を起動し直します（adj hub --new）')}">hub をリセット…</button>
  </div>`;
  return html;
}

/* ── A session with no task in the panel ────────────────────────────────────────────────────
   `selectedTaskId` is `session:<id>`: the session is the terminal, and 詳細 is what the Sessions
   list used to show beside it (sessDetailHtml). Once it is linked to a task the panel shows that. */
function sessPanelHeadHtml(s) {
  const st = sessionState(s);
  const label = sessionLabel(s, true);
  const b = boardOfSession(s);
  const hubName = b.hub ? hubLabel(b.hub) : `${b.hubId}（一覧にありません）`;
  const what = s.task ? 'タスクが見つからない' : 'タスクなし';
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(s.id)}">${esc(label.tag || 'セッション')}</span>
        ${hubName ? `<span class="origin-chip" title="${esc(hubName)}"><span class="material-symbols-outlined" aria-hidden="true">${b.hub?.parent ? 'account_tree' : 'folder'}</span><span>${esc(hubName)}</span></span>` : ''}
        <span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(STATE_LABEL[st])}</span>
        <span class="m3-pill pill-neutral">${esc(what)}</span>
      </div>
      <h2 class="tp-title" title="${esc(s.branch ? `ブランチ: ${s.branch}` : '')}">${esc(label.text)}</h2>
    </div>
    ${panelBtnsHtml('')}`;
}

/* ── ターミナル ── */
/* The terminal lives in #tp-term-host from its first mount until the task changes or the panel
   closes. 詳細 only hides the pane around it, so the socket survives; the bar and the note
   over it are the parts that are drawn again. */
function syncPanelTerminal(subject, s, pane) { syncTermSlot(panelTerm, subject, s, pane); }
function disposePanelTerminal() { disposeTermSlot(panelTerm); }

/* A slot is one terminal's place: where it mounts (`host`), what to draw again when it ends
   (`redraw`) and which board's path it connects through (`base`). The task panel has one and the
   review view another, and both keep the socket across a redraw the same way. */
function syncTermSlot(slot, subject, s, pane) {
  // Another task's (or hub's) socket is not carried over; a fresh one is asked for after
  // 再開 or 再接続. A session that was linked to a task keeps its own: only the subject's name
  // changed.
  const sameSession = !!slot.term && !!s && s.id === slot.sessionId;
  if ((slot.taskId !== subject && !sameSession) || (slot.term && s && s.id !== slot.sessionId)
      || (slot.reconnect && boardTerminalReady(s))) disposeTermSlot(slot);
  slot.taskId = subject;
  if (pane !== 'term' || slot.term || !boardTerminalReady(s)) return;
  slot.sessionId = s.id;
  slot.ended = null;
  const handle = mountSessionTerminal(slot.host(), {
    sessionId: s.id,
    base: slot.base(),
    onEnd: code => {
      if (slot.term !== handle) return;
      slot.ended = code;
      keepScreen({ sessionId: s.id, term: handle });
      slot.redraw();
    },
  });
  slot.term = handle;
}

function disposeTermSlot(slot) {
  const { term, sessionId } = slot;
  if (term) {
    keepScreen({ sessionId, term });
    term.dispose();
  }
  Object.assign(slot, { taskId: null, sessionId: null, term: null, ended: null, reconnect: false });
}

function termBarHtml(s, slot = panelTerm, actions = true, task = null) {
  if (!s || !hasSession(s)) return '';
  const st = sessionState(s);
  const last = s.present ? lastOutputText(s) : null;
  const again = slot.term && slot.ended != null && boardTerminalReady(s);
  const reset = actions ? hubResetButton(s) : null;
  return `<span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(STATE_LABEL[st])}</span>`
    + ghBarLinksHtml(task)
    + (last ? `<span class="tp-muted">最後の出力: ${esc(last)}</span>` : '')
    + `<span class="tp-bar-gap"></span>`
    + (reset ? actionButtonHtml(reset) : '')
    + (again ? '<button type="button" class="btn-m3-tonal sess-act" data-tp-reconnect><span class="material-symbols-outlined" aria-hidden="true">sync</span><span>再接続</span></button>' : '')
    + (actions ? sessionButtons(s).bar.map(b => actionButtonHtml(b)).join('') : '');
}

/* Over the host while there is no terminal in it: why not, and the last screen if this page saw
   one. */
function termPlaceholderHtml(s) {
  if (!hasSession(s)) {
    return `<div class="tp-ph-head">セッションはありません</div>${s?.worktree ? `<div class="tp-muted">${esc(s.worktree)}${s.branch ? `（${esc(s.branch)}）` : ''}</div>` : ''}`;
  }
  if (boardTerminalReady(s)) return '<div class="tp-muted">接続しています…</div>';
  const st = restingState(s);
  const head = st === 'stopped' ? 'セッションは止まっています' : st === 'ended' ? 'セッションは終了しています'
    : !s.present ? 'このセッションは動いていません'
    : 'このセッションの端末はボードから開けません（tmux で動いているセッションだけ開けます）';
  const shot = sessView.screens[s.id];
  const mins = shot && state.now != null && shot.at != null ? minutesSince(shot.at, state.now) : null;
  const resume = canResume(s) ? '' : !s.present && s.kind === 'worker'
    ? `<div class="tp-muted">${esc(!s.conversation ? '保存された会話がないため再開できません' : state.sessionResume?.reason || 'ボードからは再開できません')}</div>` : '';
  return `<div class="tp-ph-head">${esc(head)}</div>`
    + (shot?.lines.length ? `<div class="tp-muted">このページで最後に見た画面${mins == null ? '' : `（${esc(agoLabel(mins))}）`}</div><pre class="sess-last-out">${esc(shot.lines.join('\n'))}</pre>` : '')
    + resume;
}

/* Events are heard on the panel itself: its parts are drawn again, it is not. */
tp('task-panel').addEventListener('click', e => {
  const hub = hubOfRef(selectedTaskId);
  const task = hub || isHubRef(selectedTaskId) ? null : taskById(selectedTaskId);
  const sess = !hub && !task ? sessOfRef(selectedTaskId) : null;
  if (!task && !hub && !sess) return;
  const hit = sel => e.target.closest(sel);
  let b;
  if ((b = hit('[data-pane]'))) { if (!b.disabled) go({ pane: b.dataset.pane }, { replace: true }); return; }
  if (hit('[data-tp-close]')) {
    return closeTaskPanel();
  }
  if ((b = hit('[data-tp-place]'))) return placePanel(b.dataset.tpPlace);
  if (hub) return hubPanelClick(e, hub);
  if (sess) return sessPanelClick(e, sess);
  if (hit('[data-tp-jump]')) {
    // A dialog covers the card, so it closes first; a docked panel stays beside it.
    const id = task.id;
    if (prefs.panelDialog) closeTaskPanel(); else renderTaskPanel();
    return jump('agent', id);
  }
  if ((b = hit('[data-tp-act]'))) return act(b.dataset.tpAct, task.id);
  if ((b = hit('[data-record]'))) return openRecord(b.dataset.record);
  if ((b = hit('[data-judge]'))) return judgeGate(b.dataset.judge);
  // A gate in 経過 opens where the task view reads it, rather than being repeated here.
  if ((b = hit('[data-open]'))) {
    const g = gatesOf(task).find(x => x.id === b.dataset.open);
    if (g) openTask(task.id, TAB_OF_KIND[g.kind] || 'history', g.id);
    return;
  }
  if ((b = hit('[data-history]'))) return openTask(b.dataset.history, 'history');
  if ((b = hit('[data-ide]'))) return worktreeAct('ide', b.dataset.ide);
  if ((b = hit('[data-findings]'))) return loadFindings(b.dataset.findings);
  if ((b = hit('[data-relay]'))) return relayPicked(b.dataset.relay);
  if ((b = hit('[data-sess-act]'))) {
    const s = sessionOfTask(task);
    if (!s || b.disabled) return;
    // A session resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, s);
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
});
/* A session with no task: its link and git buttons, its gate, and the terminal bar's (the
   session's own `runSessionAction`, as for a task). */
function sessPanelClick(e, s) {
  const hit = sel => e.target.closest(sel);
  let b;
  if ((b = hit('[data-side-act]'))) {
    if (b.disabled) return;
    const act = b.dataset.sideAct;
    if (act === 'git-refresh') {
      ensureGit(s, true);
      return renderTaskPanel();
    }
    if (act === 'link-new' || act === 'link-existing') return openLinkDialog(s, act === 'link-new' ? 'new' : 'existing');
    if (act === 'goto-board' && s.task) return go({ board: b.dataset.board, view: 'sessions', task: s.task, pane: 'term' });
    return;
  }
  if (hit('[data-tp-sess-gate]') && s.waiting) return goToGate(s.waiting.id, s.waiting.slug);
  if ((b = hit('[data-sess-act]'))) {
    if (b.disabled) return;
    // A session resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, s);
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
}
/* The hub's own buttons. The terminal bar's are the session's (`runSessionAction`), as for a task. */
function hubPanelClick(e, h) {
  const hit = sel => e.target.closest(sel);
  let b;
  const waiting = hubSessionOf(h).waiting;
  if (hit('[data-tp-sess-gate]') && waiting) return goToGate(waiting.id, waiting.slug);
  if ((b = hit('[data-tp-hub]'))) {
    if (b.disabled) return;
    switch (b.dataset.tpHub) {
      case 'next': return nudgeHub(hubBaseOf(h));
      case 'sync': return refreshAll(hubBaseOf(h));
      case 'start': return hubStart(h.id);
      case 'stop': return openHubStopDialog(h.id, 'stop');
      case 'close': return openHubStopDialog(h.id, 'close');
      case 'reset': return openHubStopDialog(h.id, 'reset');
      case 'restart': return openHubStopDialog(h.id, 'restart');
    }
    return;
  }
  if ((b = hit('[data-tp-task]'))) return openTaskPanel(b.dataset.tpTask);
  if (hit('[data-tp-board]')) {
    // The hub's board, with the hub still in the panel on the tab it was on; a dialog would cover
    // the board, so it closes instead.
    if (prefs.panelDialog) {
      hideTaskPanelState();
      return go({ board: h.slug, task: null, pane: 'detail' });
    }
    return go({ board: h.slug, task: selectedTaskId, pane: nav.pane });
  }
  if ((b = hit('[data-sess-act]'))) {
    if (b.disabled) return;
    // A hub started or resumed from here is connected to once its window exists (syncPanelTerminal).
    if (b.dataset.sessAct === 'resume' || b.dataset.sessAct === 'hub-start') panelTerm.reconnect = true;
    return runSessionAction(b.dataset.sessAct, hubSessionOf(h));
  }
  if (hit('[data-tp-reconnect]')) {
    panelTerm.reconnect = true;
    renderTaskPanel();
  }
}
tp('task-panel').addEventListener('change', e => {
  const b = e.target.closest('[data-relay-pick]');
  if (!b) return;
  if (b.checked) relay.picked.add(b.dataset.relayPick); else relay.picked.delete(b.dataset.relayPick);
  renderTaskPanel();
});
// Clicking outside the dialog closes the panel; the dialog mode stays.
tp('tp-scrim').addEventListener('click', () => closeTaskPanel());

/* The width is dragged from the edge that faces the page, saved when the pointer is let go;
   the arrow keys move it a step at a time. */
function setPanelWidth(raw) {
  const rail = tp('nav-rail').offsetWidth;
  prefs.panelWidth = Math.round(Math.max(320, Math.min(raw, innerWidth - rail - 320)));
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
}
tp('tp-resize').addEventListener('keydown', e => {
  if ((e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') || prefs.panelDialog || matchMedia('(max-width: 720px)').matches) return;
  e.preventDefault();
  // The panel grows away from its side: toward the right on the left, toward the left on the right.
  const grow = (e.key === 'ArrowRight') === (prefs.panelDock !== 'right');
  setPanelWidth(tp('task-panel').offsetWidth + (grow ? 24 : -24));
  savePrefs();
});
tp('tp-resize').addEventListener('pointerdown', e => {
  if (prefs.panelDialog || matchMedia('(max-width: 720px)').matches) return;
  e.preventDefault();
  const handle = e.currentTarget;
  handle.setPointerCapture(e.pointerId);
  document.body.classList.add('tp-dragging');
  const move = ev => {
    const rail = tp('nav-rail').offsetWidth;
    setPanelWidth(prefs.panelDock === 'right' ? innerWidth - ev.clientX : ev.clientX - rail);
  };
  const end = () => {
    handle.removeEventListener('pointermove', move);
    handle.removeEventListener('pointerup', end);
    handle.removeEventListener('pointercancel', end);
    document.body.classList.remove('tp-dragging');
    savePrefs();
  };
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
});

/* The hand-over form of a backlog task. The board redraws once a minute and on every change of
   state, and a textarea built again loses what is typed into it, the caret, and an IME
   composition in progress. So the form is built again only for another task, or when the saved
   instruction changed while the box is neither typed in nor focused. When it changed while the
   box is being written, the box is kept and a note says so, with a button to load the new one. */
function renderHandForm(task) {
  const el = document.getElementById('tp-form');
  if (!el) return;
  if (!task) {
    el.replaceChildren();
    delete el.dataset.task;
    return;
  }
  const saved = task.instruction || '';
  const kept = el.querySelector('#tp-instruction');
  if (kept && el.dataset.task === task.id) {
    // Compared with what the box showed, not the saved string: the parser drops a leading newline
    // and turns CRLF into LF, so the saved string can differ from an untouched box.
    const writing = kept.value !== el.dataset.shown || document.activeElement === kept;
    if (el.dataset.saved === saved || writing) {
      el.querySelector('[data-stale]').hidden = el.dataset.saved === saved;
      return;
    }
  }
  el.dataset.task = task.id;
  el.dataset.saved = saved;
  el.innerHTML = `
      <div class="m3-filled-card" style="display:flex;flex-direction:column;gap:8px;">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;">キューへの受け渡し</div>
        <label for="tp-instruction" style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);">エージェントへの申し送り（指示）</label>
        <div data-stale hidden style="font-size:11.5px;color:var(--md-sys-color-error);">
          保存済みの申し送りが別の所で変わりました。いまの入力のまま渡すと上書きします。
          <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;" data-reload>変わった内容を読み込む</button>
        </div>
        <textarea id="tp-instruction" placeholder="追加の指示や申し送りがあれば入力（任意）..." style="width:100%;box-sizing:border-box;border-radius:var(--md-shape-corner-xs);border:1px solid var(--md-sys-color-outline-variant);padding:8px 10px;background:var(--md-sys-color-surface-container-high);color:var(--md-sys-color-on-surface);font-size:12.5px;font-family:inherit;resize:vertical;min-height:60px;">${esc(saved)}</textarea>
        <button class="btn-m3-primary" style="width:100%" data-tp-hand="${esc(task.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">send</span>
          <span>待機キューに渡す</span>
        </button>
      </div>
    `;
  const textarea = el.querySelector('#tp-instruction');
  el.dataset.shown = textarea.value;
  el.querySelector('[data-reload]').addEventListener('click', () => {
    const now = (state.tasks || []).find(t => t.id === task.id);
    if (!now) return;
    el.replaceChildren();
    renderHandForm(now);
  });
  el.querySelector('[data-tp-hand]').addEventListener('click', () =>
    hand(task.id, textarea.value.trim()));
  textarea.addEventListener('keydown', (e) => {
    // keyCode 229 too: where compositionend comes first, the keydown that confirms the IME
    // text already has isComposing false.
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      hand(task.id, textarea.value.trim());
    }
  });
}

registerView('task-panel', { render: () => renderTaskPanel(), reset: () => hideTaskPanelState() });
