/* The right sidebar of the セッション view: what the selected session is working on, read from
   the task behind it, the worktree under it and the hub above it. Shown or hidden as a whole.
   It draws with what the board already has (the timeline of the task view, the hand-over of
   the board), and reads the board of another hub when the session belongs to one. */

/* Wide enough, the sidebar is a column of its own and the choice is kept; narrower, it floats
   over the terminal, starts hidden and is never saved. */
const narrowSide = matchMedia('(max-width: 1399px)');
const sideVisible = () => narrowSide.matches ? sessView.sideNarrowOpen : prefs.sessionsSide !== 'closed';

function toggleSide() {
  if (narrowSide.matches) {
    sessView.sideNarrowOpen = !sessView.sideNarrowOpen;
  } else {
    prefs.sessionsSide = sideVisible() ? 'closed' : 'open';
    savePrefs();
  }
  renderSessionSidebar();
}
sessEl('sess-side-toggle').addEventListener('click', toggleSide);
sessEl('sess-side-close').addEventListener('click', toggleSide);
// A window resized across the line changes what "shown" means; a floating sidebar that
// carried over would cover the terminal without anyone asking for it.
narrowSide.addEventListener('change', () => {
  sessView.sideNarrowOpen = false;
  renderSessionSidebar();
});

/* ── The board a session's task is on ── */

/* What changes the sidebar's answer: a board is read again when this does, and otherwise only
   by the timer below. */
const sideSignal = s => JSON.stringify([s.task, s.phase, s.phaseAt, s.waiting?.id, s.present,
  (state.hubs || []).find(h => h.id === hubOfSession(s))?.inboxCount]);

/* Another board's state, kept by slug. A second request made while one is out is dropped
   unless `force`, and an answer is used only if no newer request has been made since. */
function loadSideBoard(slug, base, key, force = false) {
  const prev = sessView.boards[slug];
  if (prev?.loading && !force) return;
  const entry = sessView.boards[slug] = { data: prev?.data || null, at: prev?.at || null, key: key ?? prev?.key ?? null, loading: true, error: null };
  boardApi(base, '/api/state').then(data => {
    if (sessView.boards[slug] !== entry) return;
    Object.assign(entry, { data, at: Date.now(), loading: false });
  }).catch(e => {
    if (sessView.boards[slug] !== entry) return;
    Object.assign(entry, { loading: false, error: e.message === '404' || e.message === 'no such board' ? 'この hub のボードが見つかりません' : e.message });
  }).then(() => {
    if (view !== 'sessions') return;
    renderSessionContext();
    renderSessionSidebar();
  });
}

/* After an action on another board: its state read again at once. */
function refetchSideBoard(base) {
  if (base === BASE) return;
  const slug = base.replace(/^\/b\//, '');
  const s = currentSession();
  loadSideBoard(slug, base, s ? sideSignal(s) : null, true);
}

/* The board of the selected session, as the sidebar reads it: this page's own state, or the
   last answer of the other board (null while it is still out, or when it failed). */
function sideBoard(s) {
  const b = boardOfSession(s);
  if (b.own || !b.slug) return { data: state, base: BASE, own: true, hub: b.hub, hubId: b.hubId, slug: b.slug };
  const entry = sessView.boards[b.slug];
  return { data: entry?.data || null, base: b.base, own: false, hub: b.hub, hubId: b.hubId, slug: b.slug, error: entry?.error || null };
}

/* Only a session that has a task, or a parent-task hub, shows anything from its board. */
const sideNeedsBoard = (s, b) => s.kind === 'hub' ? !!b.hub?.parent : !!s.task;

// Another board changes without this page's state changing, so the selected session's board is
// read on a timer too, while the sidebar is on screen.
setInterval(() => {
  if (view !== 'sessions' || !sideVisible()) return;
  const s = currentSession();
  if (!s) return;
  const b = boardOfSession(s);
  if (!b.own && b.slug && sideNeedsBoard(s, b)) loadSideBoard(b.slug, b.base, sideSignal(s));
}, 10000);

/* ── The worktree's git state ── */

const gitSig = s => JSON.stringify([s.id, s.phase, s.phaseAt, s.present, s.branch]);

/* Read on selection, and again when the session moves on (a phase, a stop, another branch),
   never on a timer: the check runs git in the worktree. Said again by the 更新 button. */
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
    if (sessView.git === mine) renderSessionSidebar();
  });
}

/* `M/D HH:MM` for epoch seconds, in the reader's own time. */
function clock(secs) {
  const d = new Date(secs * 1000);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
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
      up.count > 0 ? `push していない ${up.count} 件${up.against ? `（${up.against} と比べて）` : ''}` : 'push していないコミットなし',
      m.merged === true ? `${m.ref || m.base} にマージ済み`
        : m.merged === false ? `${m.ref || m.base || 'ベース'} に未マージ`
          : `マージ済みかは判定できません${m.reason ? `（${m.reason}）` : ''}`,
      d.head ? `HEAD ${d.head}` : null,
    ].filter(Boolean);
    body = items.map(t => `<li>${esc(t)}</li>`).join('');
    if (g.error) body += `<li class="warn">${esc(`更新できませんでした: ${g.error}`)}</li>`;
  }
  const mins = g.at != null ? Math.max(0, Math.floor((Date.now() - g.at) / 60000)) : null;
  const when = mins == null ? '' : mins < 1 ? 'たった今確認' : `${minutesLabel(mins)}前に確認`;
  return `<ul class="sess-side-facts">${body}</ul>` +
    `<div class="source">${esc(when)} <button type="button" class="btn-m3-text sess-side-btn" data-side-act="git-refresh"${g.loading ? ' disabled' : ''}>更新</button></div>`;
}

/* ── Pieces ── */

const sideSection = (title, inner) => `<section class="sess-side-sec"><h3>${esc(title)}</h3>${inner}</section>`;
const sideRows = rows => `<dl class="kv">${rows.filter(Boolean).map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('')}</dl>`;
const sideNote = text => `<div class="source">${esc(text)}</div>`;
const sideMono = t => `<span class="mono2">${esc(t)}</span>`;
const sideLink = (url, text) => httpUrl(url)
  ? `<a href="${esc(url)}" target="_blank" rel="noopener noreferrer">${esc(text)}</a>` : esc(text);
const sideBtn = (act, label, attrs = '') =>
  `<button type="button" class="btn-m3-text sess-side-btn" data-side-act="${act}"${attrs}>${esc(label)}</button>`;

const SIDE_STATUS_LABEL = { backlog: 'Backlog', queued: '待ち' };

/* Where a card sits: the status column while it has not started, else who holds the ball. */
function sideColumns(t, data) {
  const human = humanColOf(t, data);
  const agent = SIDE_STATUS_LABEL[t.status] || agentLabel(agentColOf(t, data));
  return { agent, human: human ? humanLabel(human) : null };
}

/* What the PR is waiting for, from the record alone (no call to GitHub). */
function sidePrState(task, s) {
  if (task.status === 'done') return '完了';
  if (task.status === 'cancelled') return '取り消し';
  if (s.phase === 'pr') return 'レビュー待ち';
  if (s.phase === 'pr-bots') return 'bot待ち';
  if (s.phase === 'review') return 'レビュー対応中';
  return 'PR あり';
}

/* The session's phases with the time spent in each, the current one last. */
function sidePhasesHtml(s) {
  const list = s.phases?.length ? s.phases : s.phase && s.phaseAt != null ? [[s.phase, s.phaseAt]] : [];
  if (!list.length) return '';
  const rows = list.map(([phase, at], i) => {
    const last = i === list.length - 1;
    const end = last ? state.now : list[i + 1][1];
    // A worker that is gone has no "until now" to count to.
    const mins = end != null && (!last || s.present) ? Math.max(0, Math.floor((end - at) / 60)) : null;
    return `<li><span class="at">${esc(clock(at))}</span><div class="what"><div>${esc(PHASE_LABEL[phase] || phase)}` +
      `${mins == null ? '' : `<span class="who"> ${esc(minutesLabel(mins))}${last ? '（いま）' : ''}</span>`}</div></div></li>`;
  });
  return `<ol class="timeline">${rows.join('')}</ol>`;
}

/* A task of a board's own starts on a button when nothing is working on it. `hub` is the hub
   of that board; its absence leaves the queue alone. */
function sideChildButton(t, b) {
  if (t.status === 'done' || t.status === 'cancelled' || workerOf(t, b.data)) return '';
  const h = b.hub;
  // The same refusals as the hub's own row: a hub that runs needs no start.
  const stopped = !!h && !h.state?.present;
  const why = !stopped ? ''
    : h.parent && !h.key ? 'キーが分からないため起動できません。adj hub --hub <キー> で起動してください'
      : !state.hubStart?.available ? 'ボードからの起動は terminal.preset が "tmux" のときだけ使えます' : '';
  const off = why ? ` disabled title="${esc(why)}"` : '';
  if (t.status === 'backlog') return sideBtn('child-hand', '着手を依頼…', ` data-id="${esc(t.id)}"${off}`);
  if (t.status === 'queued') {
    if (stopped) return sideBtn('child-start-hub', 'hub を起動', ` data-id="${esc(t.id)}"${off}`);
    return sideBtn('child-nudge', '待ちの先頭を着手', ` data-id="${esc(t.id)}" title="hub に、worker の枠が空いていれば待ちの先頭のタスクを着手するよう頼みます（この子とは限りません）"`);
  }
  return '';
}

function sideChildrenHtml(tasks, b) {
  if (!tasks.length) return sideNote('子タスクはありません');
  return `<ul class="sess-side-list">${tasks.map(t => {
    const col = sideColumns(t, b.data);
    return `<li><div class="sess-side-child"><button type="button" class="linkish" data-side-act="child-card" data-id="${esc(t.id)}">${esc(t.title || t.id)}</button>` +
      `<span class="who">${esc(col.agent)}${col.human ? ` ・ ${esc(col.human)}` : ''}</span></div>${sideChildButton(t, b)}</li>`;
  }).join('')}</ul>`;
}

function sideTaskHtml(s, task, b) {
  const { data, base } = b;
  const cols = sideColumns(task, data);
  const issueKey = task.issue || (task.issueUrl ? `#${issueNumberOf(task.issueUrl)}` : '');
  const parent = task.parent ? sideLink(task.parent, task.parent)
    : b.hub?.parent ? esc(`親タスク ${hubShortName(b.hub)}`) : '';
  let h = sideSection('タスク', sideRows([
    ['タイトル', esc(task.title || task.id)],
    issueKey ? ['Issue', sideLink(task.issueUrl, String(issueKey))] : ['タスク ID', sideMono(task.id)],
    parent ? ['親タスク', parent] : null,
    ['エージェント側', esc(cols.agent)],
    ['人側', esc(cols.human || '—')],
  ]) + `<div class="sess-side-actions">${sideBtn('card', 'カードを開く')}</div>`);

  const all = gatesOf(task, data, base);
  h += sideSection('確認と回答', timelineHtml(task, all, 5, data, base) +
    `<div class="sess-side-actions">${sideBtn('history', '経過をすべて見る →')}</div>`);

  const phases = sidePhasesHtml(s);
  h += sideSection('フェーズ', (phases || sideNote('まだフェーズの記録はありません')) + sideRows([
    ['完了条件', esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')],
    ['止める所', esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt)],
  ]));

  if (task.pr) {
    const n = prNumberOf(task.pr);
    h += sideSection('PR', sideRows([['PR', sideLink(task.pr, n ? `#${n}` : 'PR を開く')], ['状態', esc(sidePrState(task, s))]]));
  }

  const git = sessView.git?.id === s.id ? sessView.git.data : null;
  h += sideSection('ブランチと worktree', sideRows([
    ['ブランチ', (s.branch || task.branch) ? sideMono(s.branch || task.branch) : '—'],
    ['ベース', sideMono(git?.merged?.base || task.base || '既定のベース')],
    (s.worktree || task.worktree) ? ['worktree', sideMono(s.worktree || task.worktree)] : null,
  ]) + gitFactsHtml(s));

  h += sideSection('子タスク', sideChildrenHtml(sideChildTasks(task, b), b));

  if (task.note) h += sideSection('ノート', `<div class="sess-side-pre">${esc(task.note)}</div>`);

  const snap = task.issueSnapshot;
  const issueRef = task.issueUrl || task.issue;
  if (snap) {
    h += sideSection('Issue の本文', `<div><b>${esc(snap.title)}</b></div>` +
      (snap.body ? `<div class="sess-side-issue">${esc(snap.body)}</div>` : sideNote('本文なし')) +
      `<div class="sess-side-actions">${httpUrl(snap.url) ? `<a href="${esc(snap.url)}" target="_blank" rel="noopener noreferrer">Issue で続きを読む</a>` : ''}` +
      `${sideBtn('fetch-issue', '再取得')}</div>` +
      sideNote(`${when(snap.fetchedAt)}（${ago(snap.fetchedAt)}）に取得${snap.truncated ? '。先頭のみ保存' : ''}`));
  } else if (isGithubIssue(issueRef)) {
    h += sideSection('Issue の本文', sideNote('まだ取得していません') + `<div class="sess-side-actions">${sideBtn('fetch-issue', '本文を取得')}</div>`);
  }
  return h;
}

/* The tasks that share a parent with this one: every other task on a parent-task hub's board,
   and on any board those naming the same parent. */
function sideChildTasks(task, b) {
  const tasks = b.data?.tasks || [];
  return tasks.filter(t => t.id !== task.id && (b.hub?.parent || (task.parent && t.parent === task.parent)));
}

function sideTmuxText(s) {
  const t = s.terminal;
  return t?.backend === 'tmux' ? `${t.session || ''}:${t.window || ''}` : '';
}

function sideHubHtml(s, h, b) {
  let html = '';
  if (h) {
    const inbox = h.inbox || [];
    const rows = inbox.map(m => `<li><div>${esc(m.subject || m.name)}</div>` +
      `<div class="who">${esc([m.kind, m.from, m.at ? when(m.at) : ''].filter(Boolean).join(' ・ '))}</div></li>`).join('');
    const more = h.inboxCount > inbox.length ? sideNote(`ほか ${h.inboxCount - inbox.length} 件`) : '';
    html += sideSection('受信箱', rows ? `<ul class="sess-side-list">${rows}</ul>${more}` : sideNote('受信箱は空です'));
    const workers = (state.sessions || []).filter(w => w.kind === 'worker' && w.hub === h.id);
    html += sideSection('起動した worker', workers.length
      ? `<ul class="sess-side-list">${workers.map(w => `<li><button type="button" class="linkish" data-sid="${esc(w.id)}">${esc(sessionKey(w))}</button>` +
        `<span class="who">${esc(STATE_LABEL[sessionState(w)])}</span></li>`).join('')}</ul>`
      : sideNote('この hub が起動した worker はありません'));
    if (h.parent) {
      html += sideSection('子タスク', b.data ? sideChildrenHtml(b.data.tasks || [], b)
        : sideNote(b.error || '読み込んでいます…'));
    }
  } else {
    html += sideSection('hub', sideNote('この hub の記録は一覧にありません'));
  }
  const name = h?.name || '';
  const runner = (state.hubRunner || '').replaceAll('{name}', name);
  html += sideSection('実行コマンド', sideRows([
    ['起動', sideMono(h?.parent && h.key ? `adj hub --hub ${h.key}` : 'adj hub')],
    ['ランナー', runner ? sideMono(runner) : '—'],
    ['エージェント', s.agent ? esc(s.agent) : null],
    sideTmuxText(s) ? ['tmux', sideMono(sideTmuxText(s))] : null,
  ]));
  return html;
}

function sideBranchHtml(s) {
  return sideSection('ブランチと worktree', sideRows([
    ['ブランチ', s.branch ? sideMono(s.branch) : '—'],
    s.worktree ? ['worktree', sideMono(s.worktree)] : null,
  ]) + gitFactsHtml(s));
}

/* A session with no task, a worktree with no session, or a task this board does not have:
   what it is, and what it can become. */
function sideBareHtml(s, st, b, missing) {
  let what;
  if (missing) what = 'タスク ID はこのボードに見つかりません';
  else if (st === 'none') what = 'セッションのない worktree';
  else what = 'タスクのないセッション';
  let html = sideSection('このセッション', sideNote(what) + (missing || st === 'none' ? '' :
    sideNote(b.hub?.parent
      ? `タスクにすると親タスク ${hubShortName(b.hub)} の子として、その hub のボードに作られます`
      : 'タスクにするとリポジトリのボードに作られます')));
  return html + sideBranchHtml(s);
}

/* ── Drawing ── */

/* Redrawn only when the markup differs, so a load that changed nothing (the timer's) leaves
   a text selection or a click in progress alone; the signature is the markup itself, so every
   input of it counts. */
function renderSessionSidebar() {
  if (view !== 'sessions') return;
  const side = sessEl('sess-side');
  const body = sessEl('sess-side-body');
  const visible = sideVisible();
  const toggle = sessEl('sess-side-toggle');
  toggle.setAttribute('aria-expanded', String(visible));
  const label = visible ? '詳細を隠す' : '詳細を表示';
  toggle.title = label;
  toggle.setAttribute('aria-label', label);
  toggle.querySelector('.material-symbols-outlined').textContent = visible ? 'right_panel_close' : 'right_panel_open';
  side.hidden = !visible;
  if (!visible) { sessView.sideSig = ''; return; }

  const id = sessView.selectedId;
  const s = currentSession() || (id && sessView.last?.id === id ? sessView.last : null);
  let html;
  if (!id) {
    html = sideNote('左の一覧からセッションを選んでください');
  } else if (!s) {
    html = sideNote('セッションが見つかりません');
  } else {
    // A session that dropped off the list has no worktree record to ask about any more.
    if (currentSession()) ensureGit(s);
    const b = sideBoard(s);
    if (!b.own && b.slug && sideNeedsBoard(s, b)) {
      // Read again when the session's own signals moved since it was last read.
      const entry = sessView.boards[b.slug];
      if (!entry || (!entry.loading && entry.key !== sideSignal(s))) loadSideBoard(b.slug, b.base, sideSignal(s));
    }
    const st = sessionState(s);
    const task = s.task && b.data ? (b.data.tasks || []).find(t => t.id === s.task) : null;
    if (s.kind === 'hub') {
      html = sideHubHtml(s, b.hub, b);
    } else if (s.task && !b.data) {
      html = sideNote(b.error || '読み込んでいます…') + sideBranchHtml(s);
    } else if (task) {
      html = sideTaskHtml(s, task, b);
    } else {
      html = sideBareHtml(s, st, b, !!s.task);
    }
  }
  const sig = JSON.stringify([id, html]);
  if (sig === sessView.sideSig) return;
  sessView.sideSig = sig;
  const top = body.scrollTop;
  body.innerHTML = html;
  body.scrollTop = top;
}

/* ── Acting ── */

/* The session's task and its board, as the sidebar drew them. */
function sideTaskOf(s) {
  const b = sideBoard(s);
  const task = s.task && b.data ? (b.data.tasks || []).find(t => t.id === s.task) : null;
  return { b, task };
}

/* A link to another board's page: the same token, since it is one per machine. */
const sideBoardUrl = (b, hash) => `${b.base}/?token=${enc(TOKEN)}${hash}`;

function openSideTask(b, task, tab) {
  if (b.own) {
    if (tab) return openTask(task.id, tab);
    setView('board');
    // selectTask toggles: a card already open would be closed by the click that asks for it.
    if (selectedTaskId !== task.id) selectTask(task.id);
    return;
  }
  location.href = sideBoardUrl(b, `#task/${enc(task.id)}${tab ? `/${tab}` : ''}`);
}

function sideChildAction(act, id, s) {
  const hubSide = sideBoard(s);
  const t = (hubSide.data?.tasks || []).find(x => x.id === id);
  if (!t) return;
  if (act === 'child-card') return openSideTask(hubSide, t);
  if (act === 'child-hand') {
    const h = hubSide.hub;
    return openHandoverDialog(id, null, {
      tasks: hubSide.data.tasks, base: hubSide.base, hubId: hubSide.hubId, hubSlug: hubSide.slug, startHub: !!h && !h.state?.present,
    });
  }
  if (act === 'child-start-hub') {
    const h = hubSide.hub;
    if (!h) return;
    return sessAct('side-hub', `hub を起動 (${hubShortName(h)})`, async () => {
      await hubStart(h.id);
      refetchSideBoard(hubSide.base);
    });
  }
  if (act === 'child-nudge') {
    const line = "adj send --kind next --from dashboard --subject 'start the next queued task if a worker slot is free'";
    return sessAct('side-nudge', '待ちの先頭を着手', async () => {
      const data = await boardApi(hubSide.base, '/api/hub/next', { method: 'POST' });
      note(line, false, '枠が空いていれば待ちの先頭を着手' + handedNote(data.handed));
      await refresh();
      refetchSideBoard(hubSide.base);
    });
  }
}

sessEl('sess-side').addEventListener('click', e => {
  const sid = e.target.closest('[data-sid]');
  if (sid) return selectSession(sid.dataset.sid);
  const open = e.target.closest('[data-open]');
  const btn = e.target.closest('[data-side-act]');
  const s = currentSession();
  if (!s) return;
  if (open) {
    const { b, task } = sideTaskOf(s);
    const g = task && gatesOf(task, b.data, b.base).find(x => x.id === open.dataset.open);
    if (!g) return;
    const tab = TAB_OF_KIND[g.kind] || 'history';
    if (b.own) return openTask(task.id, tab, g.id);
    return openSideTask(b, task, tab);
  }
  if (!btn || btn.disabled) return;
  const act = btn.dataset.sideAct;
  if (act === 'git-refresh') {
    ensureGit(s, true);
    return renderSessionSidebar();
  }
  if (act.startsWith('child-')) return sideChildAction(act, btn.dataset.id, s);
  const { b, task } = sideTaskOf(s);
  if (!task) return;
  if (act === 'card') return openSideTask(b, task);
  if (act === 'history') return openSideTask(b, task, 'history');
  // The click landed on this sidebar, but `fetchIssue` dims the button that was pressed.
  if (act === 'fetch-issue') return fetchIssue(task.id, { currentTarget: btn }, b.base);
});
