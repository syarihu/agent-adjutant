/* What the task panel's 詳細 shows of a session with no task: what it is and what it can become,
   and the branch, worktree and git state under it. A worker whose task is on the board is shown
   as that task; a task of another hub's board is read from there only for its title. */

/* ── The board a session's task is on ── */

/* Another board's state, kept by slug. A second request made while one is out is dropped
   unless `force`, and an answer is used only if no newer request has been made since. */
function loadSideBoard(slug, base, force = false) {
  const prev = sessView.boards[slug];
  if (prev?.loading && !force) return;
  const entry = sessView.boards[slug] = { data: prev?.data || null, at: prev?.at || null, loading: true, error: null };
  boardApi(base, '/api/state').then(data => {
    if (sessView.boards[slug] !== entry) return;
    Object.assign(entry, { data, at: Date.now(), loading: false });
  }).catch(e => {
    if (sessView.boards[slug] !== entry) return;
    Object.assign(entry, { loading: false, error: e.message === '404' || e.message === 'no such board' ? 'この hub のボードが見つかりません' : e.message });
  }).then(() => {
    renderSessionsView();
    renderTaskPanel();
  });
}

/* After an action on another board: its state read again at once. */
function refetchSideBoard(base) {
  if (base === BASE) return;
  loadSideBoard(base.replace(/^\/b\//, ''), base, true);
}

/* ── The worktree's git state ── */

const gitSig = s => JSON.stringify([s.id, s.phase, s.phaseAt, s.present, s.branch]);

/* Read when a session's 詳細 is shown, and again when the session moves on (a phase, a stop,
   another branch), never on a timer: the check runs git in the worktree. Said again by the 更新
   button. */
function ensureGit(s, force = false) {
  if (!s.worktree || s.kind === 'hub') return;
  const sig = gitSig(s);
  const cur = sessView.git;
  if (!force && cur && cur.id === s.id && cur.sig === sig) return;
  const mine = sessView.git = { id: s.id, sig, at: cur?.id === s.id ? cur.at : null, data: cur?.id === s.id ? cur.data : null, error: null, loading: true };
  // The page's own route: its board lists every session of the repository.
  api(`/api/sessions/${enc(s.id)}/git`).then(data => {
    if (sessView.git !== mine) return;
    Object.assign(mine, { data, at: Date.now(), loading: false });
  }).catch(e => {
    if (sessView.git !== mine) return;
    Object.assign(mine, { error: e.message, loading: false });
  }).then(() => {
    if (sessView.git === mine && selectedTaskId === SESS_REF + s.id) renderTaskPanel();
  });
}

function gitFactsHtml(s) {
  const g = sessView.git;
  if (!g || g.id !== s.id) return '';
  let body;
  if (g.error && !g.data) {
    body = `<li class="warn">${esc(`git の状態を確認できませんでした: ${g.error}`)}</li>`;
  } else if (!g.data) {
    body = '<li>確認しています…</li>';
  } else {
    const d = g.data;
    const u = d.uncommitted || {};
    const up = d.unpushed || {};
    const m = d.merged || {};
    const items = [
      u.files > 0 ? `未コミット ${u.files} ファイル（+${u.insertions} -${u.deletions}）` : '未コミットの変更なし',
      u.untracked > 0 ? `追跡外 ${u.untracked} 件` : null,
      up.count > 0 ? `push していない ${up.count} 件${up.against === 'remotes' ? '（どのリモートにもない）' : up.against === 'upstream' ? '（upstream と比べて）' : ''}` : 'push していないコミットなし',
      m.merged === true ? `${m.ref || m.base} にマージ済み`
        : m.merged === false ? `${m.ref || m.base || 'ベース'} に未マージ`
          : `マージ済みかは判定できません${m.reason ? `（${m.reason}）` : ''}`,
      d.head ? `HEAD ${d.head}` : null,
    ].filter(Boolean);
    body = items.map(t => `<li>${esc(t)}</li>`).join('');
    if (g.error) body += `<li class="warn">${esc(`更新できませんでした: ${g.error}`)}</li>`;
  }
  const mins = g.at != null ? minutesSince(g.at / 1000, Date.now() / 1000) : null;
  const when = mins == null ? '' : mins < 1 ? 'たった今確認' : `${minutesLabel(mins)}前に確認`;
  return `<ul class="sess-side-facts">${body}</ul>` +
    `<div class="source">${esc(when)} <button type="button" class="btn-m3-text sess-side-btn" data-side-act="git-refresh"${g.loading ? ' disabled' : ''}>更新</button></div>`;
}

/* What the agent's hooks say about a running session, as facts for the panel; empty for a
   session that is not running or has no row. The ledger's words are text, escaped where they
   are drawn, and never a class. */
function agentFactsHtml(s) {
  const a = s?.present ? s.agentSession : null;
  if (!a) return '';
  if (a.error) return `<div class="tp-muted">${esc(`エージェントの状態を読めません: ${a.error}`)}</div>`;
  const known = agentStateOf(s) && ledgerState(s, state);
  const mins = a.updatedAt != null && state.now != null ? minutesSince(a.updatedAt, state.now) : null;
  const doing = known === 'permission' && a.request ? monoKv('許可を求めている内容', a.request)
    : a.activity && known === 'working' ? monoKv('作業中', a.activity) : '';
  return `<div class="tp-kvs">
    ${a.model ? kv('モデル', esc(a.model)) : ''}
    ${a.contextPercent != null ? kv('コンテキスト', esc(`${a.contextPercent}%`)) : ''}
    ${kv('エージェントの状態', esc(known === 'permission' ? permissionLabel(s) : known ? STATE_LABEL[known] : a.status || '—'))}
    ${mins != null ? kv('いつから', esc(agoLabel(mins))) : ''}
    ${doing}
    ${a.pending ? kv('ターン終了（保留）', esc(a.pending)) : ''}
    ${a.subagents?.length > 0 ? kv('サブエージェント', esc(`${a.subagents.length} 件`)) : ''}
  </div>`;
}

/* ── 詳細 of a session with no task ── */

const sideNote = text => `<div class="tp-muted">${esc(text)}</div>`;
const sideBtn = (act, label, attrs = '') =>
  `<button type="button" class="btn-m3-text sess-side-btn" data-side-act="${act}"${attrs}>${esc(label)}</button>`;

/* A session with no task, a worktree with no session, or a task this board does not have:
   what it is, and what it can become. */
function sessDetailHtml(s, pane) {
  // Only while 詳細 is on screen: the check runs git in the worktree.
  if (pane === 'detail') ensureGit(s);
  const st = sessionState(s);
  const b = boardOfSession(s);
  let what;
  if (s.task) what = !b.own && b.slug ? 'このタスクは別の hub のボードにあります' : 'タスク ID はこのボードに見つかりません';
  else if (st === 'none') what = 'セッションのない worktree';
  else what = 'タスクのないセッション';
  const linkable = !s.task && st !== 'none';
  // A task of another hub's board is shown there, as openSessionRef does.
  const elsewhere = s.task && !b.own && b.slug && boards.some(x => x.slug === b.slug) ? b.slug : null;
  // Linking writes the task and tells the worker, which a session that is not running cannot read.
  const off = s.present ? '' : ' disabled title="セッションが動いていないため、タスクにも既存のタスクへの紐づけもできません"';
  const jump = !elsewhere ? '' :
    `<div class="sess-side-actions">${sideBtn('goto-board', 'そのボードで開く', ` data-board="${esc(elsewhere)}"`)}</div>`;
  const link = !linkable ? '' :
    sideNote(b.hub?.parent
      ? `タスクにすると親タスク ${hubShortName(b.hub)} の子として、その hub のボードに作られます`
      : 'タスクにするとリポジトリのボードに作られます') +
    `<div class="sess-side-actions">${sideBtn('link-new', 'タスクにする…', off)}${sideBtn('link-existing', '既存のタスクに紐づける…', off)}</div>`;
  // What the Sessions view's menu held: the worktree's own actions (the bar's are over the terminal).
  const busy = sessBusy.size > 0;
  const acts = sessionButtons(s).menu.map(m => actionButtonHtml(m, { busy })).join('');
  return `<div class="sess-detail">
    <div class="m3-filled-card">${secTitle('このセッション')}${sideNote(what)}${agentFactsHtml(s)}${jump}${link}</div>
    <div class="m3-filled-card">${secTitle('ブランチと worktree')}
      <div class="tp-kvs">${monoKv('ブランチ', s.branch || '—')}${s.worktree ? monoKv('worktree', baseName(s.worktree), s.worktree) : ''}</div>
      ${gitFactsHtml(s)}
      ${acts ? `<div class="tp-gate-actions">${acts}</div>` : ''}
    </div>
  </div>`;
}

/* The gate the session waits on is judged on its own screen, as a task's is. */
function sessGateHtml(s) {
  const w = s.waiting;
  if (!w) return '';
  const since = stampSecs(w.openedAt);
  const mins = since != null && state.now != null ? minutesSince(since, state.now) : null;
  return `<div class="m3-card-attention-box">
      <div class="tp-gate-head">
        <span class="material-symbols-outlined" style="font-size:16px;">pending_actions</span>
        <span>【${esc(kindOf(w.kind)[0])}】確認待ち</span>
        ${mins == null ? '' : `<span class="tp-gate-wait">${esc(minutesLabel(mins))}待ち</span>`}
      </div>
      ${w.title ? `<div style="font-size:12.5px;">${esc(w.title)}</div>` : ''}
      <div class="tp-gate-actions">
        <button type="button" class="btn-m3-primary" data-tp-sess-gate><span class="material-symbols-outlined" style="font-size:16px;">arrow_forward</span><span>判定画面を開く</span></button>
      </div>
    </div>`;
}
