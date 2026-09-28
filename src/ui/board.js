/* What `adj phase --set` writes, in the words a card shows. */
const PHASE_LABEL = { plan:'計画', implement:'実装', 'self-review':'セルフレビュー', verify:'動作確認',
                      pr:'PR', review:'レビュー対応', report:'報告' };
const minutesLabel = mins => mins < 1 ? '1分未満' : mins < 60 ? `${mins}分` : mins < 1440 ? `${Math.floor(mins / 60)}時間` : `${Math.floor(mins / 1440)}日`;

/* Minutes since the worker entered its phase, by the server's clock so a laptop that slept
   does not make every card look stuck. */
const phaseMinutes = w => w && w.phaseAt != null && state.now != null
  ? Math.max(0, Math.floor((state.now - w.phaseAt) / 60)) : null;

/* Minutes the worker has had the ball: since it entered its phase, or since a person last
   answered its gate, whichever is later. */
const workerMinutes = (task, w) => {
  const mins = phaseMinutes(w);
  const answered = stampSecs(task.gateAnsweredAt);
  if (mins == null || answered == null || state.now == null) return mins;
  return Math.min(mins, Math.max(0, Math.floor((state.now - answered) / 60)));
};

/* Why a card is stuck, or null. A badge rather than a column: moved to a column of its own,
   the card would lose the column that says where it got to. */
function stuckOf(task) {
  if (!['dispatched', 'pr'].includes(task.status)) return null;
  // Handed to Jules: the worker is expected to have gone, and the session is what moves the
  // card. Only a session that failed needs a person.
  if (task.julesSession) return task.jules?.state === 'FAILED' ? 'session を開いて確認' : null;
  // Bound for Jules but not with it yet: no worker is ever started in that worktree, so its
  // absence says nothing. The hub is having the plan written, or a person is reading it; with
  // no gate open for long, the planning stopped (the hub restarted, or the hand-over failed).
  if (task.executor === 'jules') {
    if (openGate(task)) return null;
    const since = Math.max(updatedMs(task), (stampSecs(task.gateAnsweredAt) || 0) * 1000);
    const mins = Math.floor((Date.now() - since) / 60000);
    const limit = state.stuckAfterMinutes;
    return limit > 0 && mins >= limit ? `計画が ${minutesLabel(mins)} 止まっています` : null;
  }
  if (!task.worktree) return null;
  const w = workerOf(task);
  // The worker is gone and nothing will move this card: the one thing a person must hear.
  // Not in the first two minutes, while a worker that was just dispatched is still opening.
  if (!w || !w.present) return Date.now() - updatedMs(task) < 120000 ? null : 'worker 停止';
  // Time only counts while the ball is the worker's. A card waiting on a person's answer to a
  // gate, or on reviewers once its PR is open, is not the worker being stuck — flagging those
  // would bury the ones that are.
  if (openGate(task) || task.status === 'pr') return null;
  const mins = workerMinutes(task, w);
  const limit = state.stuckAfterMinutes;
  if (mins != null && limit > 0 && mins >= limit) return `${minutesLabel(mins)} 同じ工程`;
  return null;
}

/* What a card says about the Jules session behind it, in the words the card shows. `working`
   comes from the server, which knows which states mean Jules is busy. */
const JULES_LABEL = { QUEUED:'待機中', PLANNING:'計画中', IN_PROGRESS:'作業中', AWAITING_PLAN_APPROVAL:'計画の承認待ち',
                      AWAITING_USER_FEEDBACK:'返事待ち', PAUSED:'一時停止', COMPLETED:'完了', FAILED:'失敗' };
/* The one rule for what a session's state reads as, for the card, the side sheet and the full
   view alike: an answer not in yet, or one that failed, is said as such rather than as a state. */
const julesText = j => j.error ? '状態を読めません' : j.checking ? '確認中' : (JULES_LABEL[j.state] || j.state || '');
function julesLine(task) {
  const j = task.jules;
  if (!j) return '';
  const text = julesText(j);
  const url = httpUrl(j.url);
  const label = `<span style="font-weight:700;">Jules ${esc(text)}</span>`;
  return `
    <div class="card-worker-status" title="${esc(j.error || `session ${j.session}`)}">
      ${j.working ? '<span class="pulse-dot"></span>' : '<span class="material-symbols-outlined" style="font-size:14px;">smart_toy</span>'}
      ${url ? `<a href="${esc(url)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" style="color:inherit;text-decoration:none;">${label}</a>` : label}
    </div>
  `;
}

/* Done cards older than this fold away. The records stay; the column is for
   what finished recently, not an archive to scroll past. */
const DONE_SHOWN_HOURS = 24;
let showOlderDone = false;

/* The buttons a card has for the worker behind it. Each runs on the server through the same
   templates the commands use; the log line says which command that was. */
async function focusHub() {
  const line = 'adj focus';
  try {
    const data = await api('/api/hub/focus', { method: 'POST', body: '{}' });
    note(line, false, data.present ? (data.ran ? 'hub のタブを前に出しました' : 'hub のタブが見つかりませんでした') : 'hub は動いていません');
  } catch (e) { note(`${line} → ${e.message}`, true); }
}

/* `confirmed` is set by the close dialog: closing stops the worker, so it is asked there first. */
async function worktreeAct(action, worktree, confirmed = false) {
  const line = { focus: `adj focus --worktree ${worktree}`, ide: `adj ide --worktree ${worktree}`,
                 close: `adj close --worktree ${worktree}` }[action];
  // With no editor configured the server can only refuse, so say how to set one instead.
  if (action === 'ide' && !ideReady()) { openIdeDialog(); return; }
  if (action === 'close' && !confirmed) { openCloseDialog(worktree); return; }
  try {
    const data = await api(`/api/worktrees/${action}`, { method: 'POST', body: JSON.stringify({ worktree }) });
    const why = action === 'focus' ? (data.present ? (data.ran ? 'タブを前に出しました' : 'worker のタブが見つかりませんでした') : 'worker は動いていません')
              : action === 'close' ? (data.closed ? 'タブを閉じました' : 'まだ閉じていません（確認待ちかもしれません）')
              : 'エディタで開きました';
    note(line, false, why);
    await refresh();
  } catch (e) { note(`${line} → ${e.message}`, true); }
}

const DONE_WHEN = { 'report-only':'調査のみ', verify:'動作確認まで', pr:'PR作成まで', review:'レビュー対応まで' };
const STOP_AT = { plan:'計画の承認だけ待つ', diff:'計画と差分レビューを待つ', all:'計画・差分・動作確認を待つ' };

/* The rules a worker's procedure stops a diff or verify gate by, in the words the board
   shows. The second value marks the two that mean something went wrong, rather than that a
   person was asked for. */
const STOP_RULE = {
  'round-limit':   ['レビューが上限ラウンドに達して must が残った', true],
  'verify-failed': ['verify が失敗して直せなかった', true],
  'manual-check':  ['人が見る確認がある', false],
  'unsure':        ['迷っている点がある', false],
  'stop-at':       ['タスクの止める所に含まれている', false],
};
const stopWhy = g => (g?.stoppedBy || []).map(r => (STOP_RULE[r] || [r])[0]);
const stopBad = g => (g?.stoppedBy || []).some(r => STOP_RULE[r]?.[1]);

/* ── records: what a worker wrote down without stopping ── */
const SEEN_KEY = () => `adj.seenRecords.${state.repo || ''}`;
let seenCache = null, seenCacheKey = null;
function seenRecords() {
  if (seenCache && seenCacheKey === SEEN_KEY()) return seenCache;
  seenCacheKey = SEEN_KEY();
  try { seenCache = new Set(JSON.parse(localStorage.getItem(SEEN_KEY()) || '[]')); }
  catch { seenCache = new Set(); }
  return seenCache;
}
const isUnread = r => !seenRecords().has(r.id);
function markSeen(id) {
  seenCache = null;
  const seen = seenRecords();
  if (seen.has(id)) return;
  seen.add(id);
  const live = new Set(allRecords().map(r => r.id));
  try { localStorage.setItem(SEEN_KEY(), JSON.stringify([...seen].filter(x => live.has(x)))); }
  catch {}
}
window.addEventListener('storage', e => {
  if (e.key !== SEEN_KEY()) return;
  seenCache = null;
  if (view === 'board') render();
  else if (view === 'task') redrawTaskView();
});
const allRecords = () => (state.tasks || []).flatMap(t => t.records || []);
const recordById = id => allRecords().find(r => r.id === id);
const recordSeq = r => +(/-record-(\d+)$/.exec(r.id)?.[1] || 1);
const recordsOf = task => [...(task.records || [])].sort((a, b) =>
  (a.openedAt || '').localeCompare(b.openedAt || '') || recordSeq(a) - recordSeq(b));

function recordSummary(r) {
  if (r.kind === 'diff') {
    const rounds = r.reviewRounds || [];
    const openMust = (r.findings || []).filter(f => f.severity === 'must' && f.outcome === 'open').length;
    const last = rounds[rounds.length - 1];
    if (openMust) return [`レビュー ${rounds.length ? `${rounds.length}R ` : ''}✗ must 残り ${openMust}件`, 'bad'];
    if (!rounds.length) return ['レビュー', ''];
    return last.must ? [`レビュー ${rounds.length}R ✗ 未収束`, 'bad'] : [`レビュー ${rounds.length}R ✓ 収束`, 'good'];
  }
  if (r.kind === 'verify') {
    const commands = r.commands || [];
    const failed = commands.filter(c => c.result === 'fail').length;
    if (!commands.length) return ['動作確認', ''];
    return failed ? [`verify ✗ ${failed}件失敗`, 'bad'] : ['verify ✓', 'good'];
  }
  return [kindOf(r.kind)[0], ''];
}

function chipsOf(task) {
  const latest = {};
  for (const r of recordsOf(task)) latest[r.kind] = r;
  const chips = [];
  for (const r of Object.values(latest)) {
    const [text, tone] = recordSummary(r);
    chips.push({ id: r.id, text: text + ((r.answers || []).length ? ' ↩' : ''), tone, unread: isUnread(r) });
    if (r.kind === 'verify' && (r.manual || []).length) {
      chips.push({ id: r.id, text: `手で見る ${r.manual.length}件`, tone: '', unread: false });
    }
  }
  return chips;
}

function openRecord(id) {
  const r = recordById(id);
  if (r && r.task) return openTask(r.task, TAB_OF_KIND[r.kind] || 'history', id);
  focused = id;
  setView('review');
  renderReview();
}

/* ── Inline reply forms on human board ── */
const openReplies = {}; // task/gate id -> 'reject' | 'changes' | 'answer' | 'ask'
const acting = new Set(); // guard against duplicate in-flight requests

async function act(action, id, choice) {
  const isFormOpen = ['reject', 'changes', 'answer', 'ask'].includes(action);
  if (isFormOpen) {
    openReplies[id] = action;
    renderColumns(true);
    const ta = document.querySelector(`textarea[data-reply="${id}"]`);
    if (ta) ta.focus();
    return;
  }
  if (action === 'cancel') {
    delete openReplies[id];
    renderColumns(true);
    return;
  }

  if (acting.has(id)) return;
  acting.add(id);

  let succeeded = true;
  const submitAnswer = async (...args) => {
    succeeded = await answer(...args);
  };

  try {
    const t = (state.tasks || []).find(x => x.id === id);
    const gate = t ? openGate(t) : (state.gates || []).find(g => g.id === id);
    const replyEl = document.querySelector(`textarea[data-reply="${id}"]`);
    const reply = replyEl ? replyEl.value.trim() : '';

    switch (action) {
      case 'start': {
        if (gate) {
          await submitAnswer('approve', undefined, gate.id, '');
        } else if (t) {
          try {
            await api(`/api/tasks/${encodeURIComponent(t.id)}`, {
              method: 'POST', body: JSON.stringify({ autoStart: true }),
            });
          } catch (e) {
            note(`adj task update --id ${t.id} → ${e.message}`, true);
            succeeded = false;
          }
        }
        break;
      }
      case 'shelve': {
        if (gate) {
          await submitAnswer('reject', undefined, gate.id, '');
        } else if (t) {
          await move(t.id, 'backlog');
        }
        break;
      }
      case 'approve': {
        const decision = gate?.kind === 'result' ? 'ack' : 'approve';
        if (gate) {
          await submitAnswer(decision, undefined, gate.id, reply);
        }
        break;
      }
      case 'choice': {
        if (gate && choice) {
          await submitAnswer('choice', choice, gate.id, reply);
        }
        break;
      }
      case 'send-reject': {
        const decision = gate?.kind === 'verify' || gate?.kind === 'diff' ? 'changes' : 'reject';
        if (gate) {
          await submitAnswer(decision, undefined, gate.id, reply || '差し戻し');
        }
        break;
      }
      case 'send-changes': {
        if (gate) {
          await submitAnswer('changes', undefined, gate.id, reply || '修正指示');
        } else if (t && reply) {
          try {
            await api(`/api/tasks/${encodeURIComponent(t.id)}`, {
              method: 'POST', body: JSON.stringify({ note: `PR指摘: ${reply}` }),
            });
          } catch (e) {
            note(`adj task update --id ${t.id} → ${e.message}`, true);
            succeeded = false;
          }
        }
        break;
      }
      case 'send-answer': {
        if (gate) {
          await submitAnswer('answer', undefined, gate.id, reply || '回答');
        }
        break;
      }
      case 'send-ask': {
        if (gate) {
          await submitAnswer('ask', undefined, gate.id, reply || '追加の質問');
        }
        break;
      }
      case 'refresh-prs': {
        await refreshPrs();
        break;
      }
    }
    if (succeeded) {
      delete openReplies[id];
      renderColumns(true);
      await refresh(true);
    }
  } finally {
    acting.delete(id);
  }
}

function humanActions(task, col, gate) {
  const open = openReplies[task.id];
  if (open) {
    const ph = open === 'answer' ? '回答を入力してください...'
             : open === 'ask' ? '追加で聞きたい内容を入力してください...'
             : open === 'changes' ? '修正指示や指摘を入力してください...'
             : '差し戻す理由を入力してください...';
    const sendLabel = open === 'answer' ? '回答する'
                    : open === 'ask' ? '追加で聞く'
                    : open === 'changes' ? '修正を指示'
                    : '差し戻す';
    return `
      <div class="hcard-reply">
        <textarea data-reply="${esc(task.id)}" placeholder="${esc(ph)}"></textarea>
        <div class="hcard-actions">
          <button type="button" class="btn-m3-primary" data-act="send-${open}" data-id="${esc(task.id)}">
            <span class="material-symbols-outlined">send</span>
            <span>${esc(sendLabel)}</span>
          </button>
          <button type="button" class="btn-m3-text" data-act="cancel" data-id="${esc(task.id)}">やめる</button>
        </div>
      </div>
    `;
  }
  let buttons = '';
  if (col === 'dispatch') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="start" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">play_arrow</span><span>着手する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="shelve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">undo</span><span>Backlog に戻す</span>
      </button>
    `;
  } else if (col === 'plan') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>承認する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">reply</span><span>差し戻す</span>
      </button>
    `;
  } else if (col === 'diff') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>承認して PR へ</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">reply</span><span>修正を指示</span>
      </button>
    `;
  } else if (col === 'verify') {
    const isResult = gate?.kind === 'result';
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">check</span><span>${isResult ? '了解' : '確認した'}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="${isResult ? 'ask' : 'reject'}" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">${isResult ? 'help' : 'reply'}</span><span>${isResult ? '追加で聞く' : '直してほしい'}</span>
      </button>
    `;
  } else if (col === 'prreview') {
    const prUrl = httpUrl(task.pr);
    buttons = `
      ${prUrl ? `<a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="btn-m3-primary" style="text-decoration:none;"><span class="material-symbols-outlined">open_in_new</span><span>GitHub で見る</span></a>` : ''}
      <button type="button" class="btn-m3-tonal" data-act="refresh-prs" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">sync</span><span>PR確認</span>
      </button>
      <button type="button" class="btn-m3-text" data-act="changes" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">rate_review</span><span>指摘をメモ</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">chat</span><span>回答する</span>
      </button>
    `;
  }
  if (gate && gate.choices && gate.choices.length) {
    buttons += gate.choices.map(c => `
      <button type="button" class="btn-m3-tonal" data-act="choice" data-choice="${esc(c.id)}" data-id="${esc(task.id)}">
        <span>${esc(c.label || c.id)}</span>
      </button>
    `).join('');
  }
  return `<div class="hcard-actions">${buttons}</div>`;
}

function humanGateActions(gate, col) {
  const open = openReplies[gate.id];
  if (open) {
    const ph = open === 'answer' ? '回答を入力してください...'
             : open === 'ask' ? '追加で聞きたい内容を入力してください...'
             : open === 'changes' ? '修正指示や指摘を入力してください...'
             : '差し戻す理由を入力してください...';
    const sendLabel = open === 'answer' ? '回答する'
                    : open === 'ask' ? '追加で聞く'
                    : open === 'changes' ? '修正を指示'
                    : '差し戻す';
    return `
      <div class="hcard-reply">
        <textarea data-reply="${esc(gate.id)}" placeholder="${esc(ph)}"></textarea>
        <div class="hcard-actions">
          <button type="button" class="btn-m3-primary" data-act="send-${open}" data-id="${esc(gate.id)}">
            <span class="material-symbols-outlined">send</span>
            <span>${esc(sendLabel)}</span>
          </button>
          <button type="button" class="btn-m3-text" data-act="cancel" data-id="${esc(gate.id)}">やめる</button>
        </div>
      </div>
    `;
  }
  let buttons = '';
  if (col === 'dispatch') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">play_arrow</span><span>着手する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">undo</span><span>Backlog に戻す</span>
      </button>
    `;
  } else if (col === 'plan' || col === 'diff') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">check</span><span>承認する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">reply</span><span>差し戻す</span>
      </button>
    `;
  } else if (col === 'verify') {
    const isResult = gate.kind === 'result';
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">check</span><span>${isResult ? '了解' : '確認した'}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="${isResult ? 'ask' : 'reject'}" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">${isResult ? 'help' : 'reply'}</span><span>${isResult ? '追加で聞く' : '直してほしい'}</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(gate.id)}">
        <span class="material-symbols-outlined">chat</span><span>回答する</span>
      </button>
    `;
  }
  if (gate.choices && gate.choices.length) {
    buttons += gate.choices.map(c => `
      <button type="button" class="btn-m3-tonal" data-act="choice" data-choice="${esc(c.id)}" data-id="${esc(gate.id)}">
        <span>${esc(c.label || c.id)}</span>
      </button>
    `).join('');
  }
  return `<div class="hcard-actions">${buttons}</div>`;
}

function humanCard(task, col) {
  const el = document.createElement('div');
  el.className = 'card hcard' + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `human-${task.id}`;
  el.dataset.id = task.id;

  const gate = openGate(task);
  const mins = waitingMinutes(task);
  const issueUrl = httpUrl(task.issueUrl);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : null;

  let why = '';
  if (gate) {
    if (gate.problem) why = gate.problem;
    else if (gate.why) why = gate.why;
    else if (stopWhy(gate).length) why = stopWhy(gate).join(' / ');
    else why = gate.title;
  } else if (col === 'prreview') {
    why = 'worker は PR を出して待っています。GitHub でレビューしてください';
  }

  const prUrl = httpUrl(task.pr);
  const prNumber = prUrl ? prNumberOf(prUrl) : null;

  const w = workerOf(task);
  const phaseStr = task.status === 'queued' ? '着手前'
    : (w?.phase ? `${PHASE_LABEL[w.phase] || w.phase}で停止中` : `${task.status}で停止中`);

  el.innerHTML = `
    <div class="card-header-row">
      ${issueNumber ? `
        <a href="${esc(issueUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="card-issue-link" title="GitHub Issue #${esc(issueNumber)} を開く">
          <span class="material-symbols-outlined" style="font-size:12px;">tag</span>
          <span>${esc(issueNumber)}</span>
        </a>
      ` : `
        <span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>
      `}
      <span class="wait-time ${waitTone(mins)}" title="待たせている時間">
        <span class="material-symbols-outlined">schedule</span>
        <span>${minutesLabel(mins)}待ち</span>
      </span>
    </div>
    <div class="title">${esc(task.title)}</div>
    ${col === 'question' ? `<div class="question-box">${esc(why)}</div>` : (why ? `<div class="why">${esc(why)}</div>` : '')}
    ${col === 'prreview' && prUrl ? `<div class="pills"><a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="m3-pill pill-blue" style="text-decoration:none;" title="PRを開く"><span class="material-symbols-outlined">merge</span>PR #${esc(prNumber || '')}</a></div>` : ''}
    ${humanActions(task, col, gate)}
    <div class="hcard-foot">
      <span>${esc(phaseStr)}</span>
      <button type="button" class="agent-back" data-jump-agent="${esc(task.id)}" title="エージェントのボードでこのカードを見る">
        <span class="material-symbols-outlined">smart_toy</span>
        <span>エージェントで見る</span>
      </button>
    </div>
  `;

  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('a') || e.target.closest('textarea') || e.target.closest('input')) return;
    if (gate) judgeGate(gate.id);
    else selectTask(task.id);
  };
  return el;
}

function humanGateCard(gate, col) {
  const el = document.createElement('div');
  el.className = 'card hcard';
  el.id = `human-${gate.id}`;
  el.dataset.id = gate.id;

  const mins = stampSecs(gate.openedAt) ? Math.max(0, Math.floor((Date.now() - stampSecs(gate.openedAt) * 1000) / 60000)) : 0;
  const wtName = gate.worktree ? gate.worktree.split('/').pop() : gate.id;
  const why = gate.problem || gate.why || stopWhy(gate).join(' / ') || gate.title;

  el.innerHTML = `
    <div class="card-header-row">
      <span class="card-task-id" title="${esc(gate.id)}">${esc(wtName)}</span>
      <span class="wait-time ${waitTone(mins)}" title="待たせている時間">
        <span class="material-symbols-outlined">schedule</span>
        <span>${minutesLabel(mins)}待ち</span>
      </span>
    </div>
    <div class="title">${esc(gate.title)}</div>
    ${col === 'question' ? `<div class="question-box">${esc(why)}</div>` : `<div class="why">${esc(why)}</div>`}
    ${humanGateActions(gate, col)}
    <div class="hcard-foot">
      <span>${esc(wtName)}</span>
      <button type="button" class="btn-m3-text" style="padding:0;font-size:11px;" data-gate="${esc(gate.id)}">詳細判定画面 →</button>
    </div>
  `;

  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('a') || e.target.closest('textarea') || e.target.closest('input')) return;
    judgeGate(gate.id);
  };
  return el;
}

function agentCard(task) {
  const el = document.createElement('div');
  const hcol = humanColOf(task);
  const compact = task.status === 'backlog' || task.status === 'done';
  const stuck = stuckOf(task);
  el.className = 'card' + (hcol ? ' waiting' : '') + (stuck && !hcol ? ' stuck' : '') + (compact ? ' compact' : '') + (selectedTaskId === task.id ? ' selected' : '');
  el.id = `agent-${task.id}`;
  el.dataset.id = task.id;

  const live = ['dispatched', 'pr'].includes(task.status);
  const worker = live ? workerOf(task) : null;

  let h = '';

  // 1. Header row
  const doneLabel = DONE_WHEN[task.doneWhen] || task.doneWhen;
  const donePillClass = task.doneWhen === 'report-only' ? 'pill-purple'
    : task.doneWhen === 'verify' ? 'pill-warn'
    : task.doneWhen === 'review' ? 'pill-blue'
    : 'pill-neutral';

  const issueUrl = httpUrl(task.issueUrl);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : null;

  h += `
    <div class="card-header-row">
      ${issueNumber ? `
        <a href="${esc(issueUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="card-issue-link" title="GitHub Issue #${esc(issueNumber)} を開く">
          <span class="material-symbols-outlined" style="font-size:12px;">tag</span>
          <span>${esc(issueNumber)}</span>
        </a>
      ` : `
        <span class="card-task-id" title="${esc(task.id)}">${esc(task.id)}</span>
      `}
      ${doneLabel ? `<span class="m3-pill ${donePillClass}">${esc(doneLabel)}</span>` : ''}
      ${task.status === 'queued' && task.order != null ? `<span class="m3-pill pill-neutral" title="キューの優先順"><span class="material-symbols-outlined" style="font-size:12px;">swap_vert</span>${task.order}</span>` : ''}
    </div>
  `;

  // 2. Title
  h += `<div class="title">${esc(task.title)}</div>`;

  // 3. Worker Status (or Jules)
  if (live && task.jules) {
    h += julesLine(task);
  } else if (worker && worker.phase) {
    const mins = phaseMinutes(worker);
    const stopped = !worker.present;
    h += `
      <div class="card-worker-status">
        ${hcol ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-warning);">pause_circle</span>'
               : stopped ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-error);">pause</span>'
               : '<span class="pulse-dot"></span>'}
        <span style="font-weight:700;${stopped && !hcol ? 'color:var(--md-sys-color-error);' : ''}">${esc(PHASE_LABEL[worker.phase] || worker.phase)}${stopped && !hcol ? ' (停止)' : ''}</span>
        ${mins != null ? `<span class="ago" style="margin-left:auto;color:var(--md-sys-color-outline);font-size:11px;">${minutesLabel(mins)}</span>` : ''}
      </div>
    `;
  }

  // 4. Metadata pills
  const metaBadges = [];
  if (task.executor === 'jules' && !task.jules) {
    metaBadges.push(`<span class="m3-pill pill-purple" title="計画の承認後に Jules へ渡す"><span class="material-symbols-outlined" style="font-size:12px;">smart_toy</span>Jules</span>`);
  }
  if (!task.autoStart && task.status !== 'done') {
    metaBadges.push(`<span class="m3-pill pill-warn" title="着手前に確認が必要"><span class="material-symbols-outlined" style="font-size:12px;">lock</span>要着手確認</span>`);
  }
  const prUrl = httpUrl(task.pr);
  if (prUrl) {
    const prNumber = prNumberOf(prUrl);
    metaBadges.push(`<a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="m3-pill pill-blue" style="text-decoration:none;" title="PRを開く (${esc(prUrl)})"><span class="material-symbols-outlined" style="font-size:12px;">merge</span>PR${prNumber ? ` #${esc(prNumber)}` : ''}</a>`);
  } else if (live && worker?.phase === 'pr') {
    metaBadges.push('<span class="m3-pill pill-neutral"><span class="material-symbols-outlined" style="font-size:12px;">hourglass_top</span>PR 作成中</span>');
  }
  if (metaBadges.length) {
    h += `<div class="pills" style="display:flex;flex-wrap:wrap;gap:4px;">${metaBadges.join('')}</div>`;
  }

  // 5. Chips (diff / verify records)
  const chips = chipsOf(task);
  if (chips.length) {
    h += `<div class="chips" style="display:flex;flex-wrap:wrap;gap:4px;">${chips.map(c => {
      const isGood = c.tone === 'good';
      const isBad = c.tone === 'bad';
      const pillClass = isGood ? 'pill-good' : isBad ? 'pill-err' : 'pill-warn';
      const icon = isGood ? 'check_circle' : isBad ? 'cancel' : 'info';
      return `<button type="button" class="m3-pill ${pillClass}" style="border:0;cursor:pointer;" data-record="${esc(c.id)}" title="${c.unread ? '未読 — ' : ''}クリックして記録を開く">
        ${c.unread ? '<span style="width:5px;height:5px;border-radius:50%;background:currentColor;display:inline-block;margin-right:2px;"></span>' : ''}
        <span class="material-symbols-outlined" style="font-size:13px;margin-right:2px;">${icon}</span>
        <span>${esc(c.text)}</span>
      </button>`;
    }).join('')}</div>`;
  }

  // 6. Note
  if (task.note) {
    if (task.status === 'done') {
      h += `<div style="font-size:11.5px;color:var(--md-sys-color-outline);overflow-wrap:anywhere;">${esc(task.note)}</div>`;
    } else {
      h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11.5px;color:var(--md-sys-color-error);font-weight:600;"><span class="material-symbols-outlined" style="font-size:14px;flex-shrink:0;">warning</span><span>${esc(task.note)}</span></div>`;
    }
  }

  // 6.5 Instruction
  if (task.instruction && task.status !== 'backlog') {
    h += `<div style="display:flex;align-items:flex-start;gap:4px;font-size:11px;color:var(--md-sys-color-primary);background:var(--md-sys-color-surface-container-high);padding:4px 8px;border-radius:var(--md-shape-corner-xs);margin-top:2px;">
      <span class="material-symbols-outlined" style="font-size:13px;flex-shrink:0;margin-top:1px;">forward_to_inbox</span>
      <span style="overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;" title="${esc(task.instruction)}">${esc(task.instruction)}</span>
    </div>`;
  }

  // 7. Stuck warning (only if NOT waiting on human)
  if (stuck && !hcol) {
    h += `<div style="display:flex;align-items:center;gap:4px;font-size:11px;color:var(--md-sys-color-error);font-weight:700;"><span class="material-symbols-outlined" style="font-size:14px;">timer</span><span>${esc(stuck)}</span></div>`;
  }

  // 8. Done completed time
  if (task.status === 'done' && task.updatedAt) {
    h += `<div style="font-size:11px;color:var(--md-sys-color-outline);">${ago(task.updatedAt)}に完了</div>`;
  }

  // 9. Footer row: slug & buttons
  const worktreeSlug = task.worktree ? task.worktree.split('/').pop() : '';
  const branchSlug = task.branch || '';
  const hasSlug = worktreeSlug || branchSlug || (!issueNumber && task.id);
  const hasButtons = task.worktree || task.status === 'backlog';

  if (hasSlug || hasButtons) {
    h += `
      <div class="card-footer-row">
        <div class="card-meta-slug" title="${esc(task.worktree || task.branch || task.id)}">
          ${worktreeSlug ? `<span class="material-symbols-outlined" style="font-size:13px;">folder_open</span><span>${esc(worktreeSlug)}</span>`
            : branchSlug ? `<span class="material-symbols-outlined" style="font-size:13px;">fork_right</span><span>${esc(branchSlug)}</span>`
            : issueNumber ? `<span class="card-task-id" style="font-size:10px;">${esc(task.id)}</span>`
            : ''}
        </div>
        <div class="card-button-row">
          ${task.worktree ? `<button type="button" class="m3-icon-button" title="ターミナルのworkerタブを前面表示" data-focus="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">terminal</span><span>ターミナル</span></button>` : ''}
          ${task.worktree ? `<button type="button" class="m3-icon-button" title="${ideTitle()}" data-ide="${esc(task.worktree)}"><span class="material-symbols-outlined" style="font-size:14px;">code</span><span>IDE</span></button>` : ''}
          ${task.status === 'backlog' ? `<button type="button" class="m3-icon-button" style="color:var(--md-sys-color-primary);" title="待ちキューへ渡す" data-hand="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span><span>渡す</span></button>` : ''}
        </div>
      </div>
    `;
  }

  // 10. Wait link if waiting on human
  if (hcol) {
    const mins = waitingMinutes(task);
    h += `
      <button type="button" class="wait-link" data-jump-human="${esc(task.id)}" title="人のボードでこのカードを開く">
        <div class="wait-link-row">
          <span class="wait-link-badge">
            <span class="material-symbols-outlined">person_alert</span>
            <span>人の確認待ち</span>
          </span>
          <span class="go">${minutesLabel(mins)}<span class="material-symbols-outlined">arrow_outward</span></span>
        </div>
        <div class="wait-link-target">${esc(humanLabel(hcol))}</div>
      </button>
    `;
  }

  el.innerHTML = h;
  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('.m3-pill') || e.target.closest('.card-issue-link') || e.target.closest('a')) return;
    selectTask(task.id);
  };
  return el;
}

let boardHeld = false;
/* Draws the columns afresh across both Human board and Agent board. */
function renderColumns(force = false) {
  const hb = document.getElementById('board-human');
  const ab = document.getElementById('board-agent');
  if (!hb && !ab) return;

  // Same rule as redrawReview: a redraw loses the IME composition being typed.
  if (!force && document.activeElement?.matches('textarea[data-reply]')) {
    boardHeld = true;
    return;
  }
  boardHeld = false;

  // Preserve scroll positions
  const scrolled = {};
  for (const cards of document.querySelectorAll('#boards .col-cards')) {
    const pane = cards.closest('.pane');
    const colId = cards.parentElement.dataset.col;
    if (pane && colId) scrolled[`${pane.id}-${colId}`] = cards.scrollTop;
  }

  // Preserve typed replies
  const drafts = {};
  document.querySelectorAll('textarea[data-reply]').forEach(a => {
    drafts[a.dataset.reply] = a.value;
  });

  const humanItems = (state.tasks || []).filter(t => humanColOf(t));
  const gateTaskIds = new Set((state.tasks || []).map(t => t.id));
  const standaloneGates = (state.gates || []).filter(g => !g.task || !gateTaskIds.has(g.task));

  // Render Human Board
  if (hb) {
    hb.innerHTML = '';
    for (const def of HUMAN_COLUMNS) {
      const tasksInCol = humanItems.filter(t => humanColOf(t) === def.id);
      const gatesInCol = standaloneGates.filter(g => gateHumanCol(g.kind) === def.id);
      const allItems = [
        ...tasksInCol.map(t => ({ isGate: false, item: t, at: waitingSinceMs(t) })),
        ...gatesInCol.map(g => ({ isGate: true, item: g, at: stampSecs(g.openedAt) ? stampSecs(g.openedAt) * 1000 : 0 })),
      ].sort((a, b) => a.at - b.at);

      const col = document.createElement('section');
      col.className = 'col human' + (allItems.length ? '' : ' empty');
      col.dataset.col = def.id;

      const headerHtml = `
        <div class="col-header">
          <div class="col-header-top">
            <div class="col-title-badge">
              <span class="material-symbols-outlined">${def.icon}</span>
              <span class="col-title-text" title="${esc(def.label)}">${esc(def.label)}</span>
              <span class="col-count-pill">${allItems.length}</span>
            </div>
            ${def.id === 'prreview' ? `<button type="button" class="col-btn-nudge" title="PRマージ済みタスクを確認" data-act="refresh-prs"><span class="material-symbols-outlined" style="font-size:13px;">sync</span><span>PR確認</span></button>` : ''}
          </div>
          ${def.hint ? `<div class="col-subtext" title="${esc(def.hint)}">${esc(def.hint)}</div>` : ''}
        </div>
      `;
      col.insertAdjacentHTML('afterbegin', headerHtml);

      const cards = document.createElement('div');
      cards.className = 'col-cards';
      if (!allItems.length) {
        cards.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder">なし</div>');
      } else {
        for (const entry of allItems) {
          if (entry.isGate) {
            cards.appendChild(humanGateCard(entry.item, def.id));
          } else {
            cards.appendChild(humanCard(entry.item, def.id));
          }
        }
      }
      col.appendChild(cards);
      hb.appendChild(col);
    }
  }

  // Stuck strip on Human Board
  const stuckSlot = document.getElementById('stuck-strip-slot');
  if (stuckSlot) {
    const activeTasks = (state.tasks || []).filter(t => ['dispatched', 'pr'].includes(t.status));
    const stuck = activeTasks.filter(t => stuckOf(t) && !humanColOf(t));
    stuckSlot.innerHTML = stuck.length
      ? `<div class="stuck-strip">
          <span class="material-symbols-outlined" style="font-size:16px;">timer</span>
          <span>止まっている worker があります：</span>
          ${stuck.map(t => {
            const w = workerOf(t);
            const p = w?.phase ? (PHASE_LABEL[w.phase] || w.phase) : t.status;
            return `<button type="button" data-jump-agent="${esc(t.id)}">${esc(t.id)}（${esc(p)}: ${esc(stuckOf(t))}）</button>`;
          }).join(' ')}
        </div>`
      : '';
  }

  // Render Agent Board
  if (ab) {
    ab.innerHTML = '';
    for (const def of AGENT_COLUMNS) {
      const items = (state.tasks || []).filter(t => agentColOf(t) === def.id);
      const isNarrow = def.id === 'before' || def.id === 'done';
      let older = [];
      let displayItems = items;
      if (def.id === 'done') {
        const cutoff = Date.now() - DONE_SHOWN_HOURS * 3600 * 1000;
        older = items.filter(t => updatedMs(t) < cutoff);
        if (!showOlderDone) displayItems = items.filter(t => updatedMs(t) >= cutoff);
      }

      const totalCount = def.id === 'done' ? (showOlderDone ? items.length : displayItems.length) : items.length;
      const isEmpty = totalCount === 0;
      const col = document.createElement('section');
      col.className = 'col' + (isNarrow ? ' narrow' : '') + (isEmpty ? ' empty' : '');
      col.dataset.col = def.id;

      const nextBtn = def.id === 'before'
        ? '<button type="button" class="col-btn-nudge" title="workerの枠が空いていれば次を着手" onclick="nudgeHub()"><span class="material-symbols-outlined" style="font-size:13px;">bolt</span><span>次を流す</span></button>'
        : '';

      const headerHtml = `
        <div class="col-header">
          <div class="col-header-top">
            <div class="col-title-badge">
              <span class="material-symbols-outlined">${def.icon}</span>
              <span class="col-title-text" title="${esc(def.label)}">${esc(def.label)}</span>
              <span class="col-count-pill">${totalCount}</span>
            </div>
            ${nextBtn}
          </div>
          ${def.hint ? `<div class="col-subtext" title="${esc(def.hint)}">${esc(def.hint)}</div>` : ''}
        </div>
      `;
      col.insertAdjacentHTML('afterbegin', headerHtml);

      const cards = document.createElement('div');
      cards.className = 'col-cards';

      if (def.id === 'before') {
        const queued = items.filter(t => t.status === 'queued').sort((a, b) => (a.order || 0) - (b.order || 0));
        const backlog = items.filter(t => t.status === 'backlog');
        cards.insertAdjacentHTML('beforeend', `<div class="col-section"><span class="material-symbols-outlined" style="font-size:14px;">hourglass_empty</span><span>待ち ${queued.length}</span></div>`);
        if (!queued.length) cards.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder" style="margin-bottom:8px;">待ちタスクなし</div>');
        for (const t of queued) cards.appendChild(agentCard(t));

        cards.insertAdjacentHTML('beforeend', `<div class="col-section" style="margin-top:10px;"><span class="material-symbols-outlined" style="font-size:14px;">inventory_2</span><span>Backlog ${backlog.length}</span></div>`);
        if (!backlog.length) cards.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder">Backlog なし</div>');
        for (const t of backlog) cards.appendChild(agentCard(t));
      } else {
        displayItems.sort((a, b) => (!!humanColOf(a) - !!humanColOf(b)) || (b.phaseAt || updatedMs(b) || 0) - (a.phaseAt || updatedMs(a) || 0));
        if (!displayItems.length) {
          cards.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder">なし</div>');
        } else {
          for (const t of displayItems) cards.appendChild(agentCard(t));
        }

        if (def.id === 'implement') {
          const known = new Set((state.tasks || []).map(t => t.worktree).filter(Boolean));
          for (const w of (state.workers || []).filter(w => w.present && !known.has(w.worktree))) {
            cards.appendChild(ghostEl(w));
          }
        }

        if (older.length && def.id === 'done') {
          cards.insertAdjacentHTML('beforeend',
            `<button type="button" class="more" style="width:100%;margin-top:6px;padding:6px;font-size:11.5px;border:1px dashed var(--md-sys-color-outline-variant);border-radius:var(--md-shape-corner-sm);background:transparent;color:var(--md-sys-color-primary);">${showOlderDone ? '以前の完了を畳む' : `以前の完了 ${older.length} 件`}</button>`);
          cards.querySelector('.more').addEventListener('click', () => { showOlderDone = !showOlderDone; render(); });
        }
      }

      col.appendChild(cards);
      ab.appendChild(col);
    }
  }

  // Restore scroll positions
  for (const cards of document.querySelectorAll('#boards .col-cards')) {
    const pane = cards.closest('.pane');
    const colId = cards.parentElement.dataset.col;
    if (pane && colId) {
      const top = scrolled[`${pane.id}-${colId}`];
      if (top) cards.scrollTop = top;
    }
  }

  // Restore typed drafts
  document.querySelectorAll('textarea[data-reply]').forEach(a => {
    if (drafts[a.dataset.reply] != null) a.value = drafts[a.dataset.reply];
  });
}

function ghostEl(worker) {
  const el = document.createElement('div');
  el.className = 'card ghost';
  el.innerHTML = `<div class="title">${esc(worker.title || worker.name)}</div>
    <div class="meta"><span>${esc(worker.branch || '')}</span>
      <span class="state warn"><span>◷</span>タスクレコードなし</span></div>
    <div class="mono">${esc(worker.worktree)}</div>`;
  return el;
}

function selectTask(id) {
  selectedTaskId = (selectedTaskId === id ? null : id);
  for (const card of document.querySelectorAll('#boards .card')) {
    card.classList.toggle('selected', card.dataset.id === selectedTaskId);
  }
  renderDrawer();
}

function closeDrawer() {
  selectedTaskId = null;
  for (const card of document.querySelectorAll('#boards .card')) {
    card.classList.remove('selected');
  }
  renderDrawer();
}

function goToGate(gateId) {
  focused = gateId;
  setView('review');
  renderReview();
}

/* Where a waiting gate is read and answered: its task's view, in the tab of its kind, when the
   task is on the board. A gate with no task — the hub's, or one whose task is gone — has no
   such view and opens in the review view. */
function judgeGate(gateId) {
  const g = (state.gates || []).find(x => x.id === gateId);
  const owner = g?.task && (state.tasks || []).find(t => t.id === g.task);
  if (owner) openTask(owner.id, TAB_OF_KIND[g.kind] || 'history', g.id);
  else goToGate(gateId);
}

/* The レビュー tab: the first gate in the queue, where it is answered; the review view when the
   queue is empty, which says so. */
function goToQueue() {
  // Already on the queue: stay on the one being read, and on whatever is typed for it.
  if (view === 'review') return;
  const first = (state.gates || [])[0];
  if (first) judgeGate(first.id);
  else setView('review');
}

/* The review comments of the task the side sheet is open on, once a person asked for them.
   Kept here rather than in /api/state: listing them is a round trip to GitHub, done when
   somebody wants to choose, and the choice has to survive the side sheet being redrawn. */
let relay = { taskId: null, loading: false, error: '', findings: [], picked: new Set() };

async function loadFindings(id) {
  const asked = { taskId: id, loading: true, error: '', findings: [], picked: new Set() };
  relay = asked;
  renderDrawer();
  let data = null, failed = null;
  try { data = await api(`/api/tasks/${encodeURIComponent(id)}/findings`); } catch (e) { failed = e; }
  // Another list was asked for meanwhile — another task's, or this one again. That one's
  // answer is the one to show; this one would put its comments under the wrong card.
  if (relay !== asked) return;
  if (failed) {
    relay.error = failed.message;
    note(`adj jules findings --id ${id} → ${failed.message}`, true);
  } else {
    relay.findings = data.findings || [];
    note(`adj jules findings --id ${id}`, false, `${relay.findings.length} 件`);
  }
  relay.loading = false;
  renderDrawer();
}

async function relayPicked(id) {
  const comments = [...relay.picked];
  // One at a time: a second click while the first is posting would only come back refused.
  if (!comments.length || relay.posting) return;
  const asked = relay;
  asked.posting = true;
  renderDrawer();
  const line = `adj jules relay --id ${id} ${comments.map(c => `--comment ${c}`).join(' ')}`;
  try {
    await api(`/api/tasks/${encodeURIComponent(id)}/relay`, { method: 'POST', body: JSON.stringify({ comments }) });
    note(line, false, `${comments.length} 件を PR にコメントしました。Jules が読んで直します`);
    // Read again only if the side sheet is still on this list; otherwise the reload would
    // replace whatever is being looked at now.
    if (relay === asked && selectedTaskId === id) await loadFindings(id);
  } catch (e) { note(`${line} → ${e.message}`, true); }
  asked.posting = false;
  if (relay === asked) renderDrawer();
}

function relayHtml(task) {
  const mine = relay.taskId === task.id;
  let h = `
    <div class="m3-filled-card">
      <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">レビュー指摘を Jules に回す</div>
      <p style="font-size:12px;color:var(--md-sys-color-on-surface-variant);margin:0 0 8px;">Jules は起動した本人以外のコメントには反応しないので、選んだ指摘をあなたの名前で PR にコメントし直します。</p>`;
  if (!mine || (!relay.loading && !relay.findings.length && !relay.error)) {
    h += `<button type="button" class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;align-self:flex-start;" data-findings="${esc(task.id)}">
      <span class="material-symbols-outlined" style="font-size:16px;">download</span><span>${mine ? '指摘はありません — 読み直す' : '指摘を読み込む'}</span></button>`;
  } else if (relay.loading) {
    h += `<div style="font-size:12px;color:var(--md-sys-color-outline);">読み込み中…</div>`;
  } else if (relay.error) {
    h += `<div style="font-size:12px;color:var(--md-sys-color-error);overflow-wrap:anywhere;">${esc(relay.error)}</div>
      <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;" data-findings="${esc(task.id)}">読み直す</button>`;
  } else {
    h += `<div style="display:flex;flex-direction:column;gap:6px;">` + relay.findings.map(f => {
      const place = f.line != null ? `${f.path}:${f.line}` : f.path;
      return `<label style="display:flex;gap:8px;align-items:flex-start;font-size:12px;${f.relayed ? 'opacity:.55;' : ''}">
        <input type="checkbox" data-relay-pick="${esc(f.id)}" ${relay.picked.has(f.id) ? 'checked' : ''} ${f.relayed ? 'disabled' : ''} style="margin-top:2px;">
        <span style="display:flex;flex-direction:column;gap:2px;min-width:0;">
          <code style="font-family:var(--font-mono);font-size:11.5px;word-break:break-all;">${esc(place)}${f.relayed ? '（回し済み）' : ''}</code>
          <span style="font-size:11px;color:var(--md-sys-color-outline);">${esc(f.author)}</span>
          <span style="color:var(--md-sys-color-on-surface-variant);overflow-wrap:anywhere;">${esc((f.text || '').split('\n')[0].slice(0, 160))}</span>
          ${httpUrl(f.url) ? `<a href="${esc(f.url)}" target="_blank" rel="noopener noreferrer" style="color:var(--md-sys-color-primary);font-size:11px;">GitHub で見る</a>` : ''}
        </span>
      </label>`;
    }).join('') + `</div>
      <div style="display:flex;gap:8px;margin-top:8px;">
        <button type="button" class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;" data-relay="${esc(task.id)}" ${relay.picked.size && !relay.posting ? '' : 'disabled'}>
          <span class="material-symbols-outlined" style="font-size:16px;">forward</span><span>選んだ ${relay.picked.size} 件を Jules に回す</span></button>
        <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;" data-findings="${esc(task.id)}">読み直す</button>
      </div>`;
  }
  return h + `</div>`;
}

function renderDrawer() {
  const drawer = document.getElementById('task-drawer');
  if (!drawer) return;
  // Closed, not just out of view: what was typed for the task goes with it.
  if (!selectedTaskId) {
    drawer.classList.add('hidden');
    renderHandForm(null);
    return;
  }
  if (view !== 'board') {
    drawer.classList.add('hidden');
    return;
  }
  const task = (state.tasks || []).find(t => t.id === selectedTaskId);
  if (!task) {
    drawer.classList.add('hidden');
    renderHandForm(null);
    selectedTaskId = null;
    return;
  }
  drawer.classList.remove('hidden');

  const colId = columnOf(task);
  const colObj = COLUMNS.find(c => c.id === colId);
  const gate = openGate(task);

  const badgesEl = document.getElementById('drawer-badges');
  if (badgesEl) {
    badgesEl.innerHTML = `
      <span class="m3-pill ${gate ? 'pill-warn' : 'pill-blue'}">${colObj ? colObj.label : task.status}</span>
      <span style="font-family:var(--font-mono);font-size:11.5px;color:var(--md-sys-color-outline);margin-left:4px">${esc(task.id)}</span>
    `;
  }
  const titleEl = document.getElementById('drawer-title');
  if (titleEl) titleEl.textContent = task.title;
  const expandBtn = document.getElementById('drawer-expand-btn');
  if (expandBtn) expandBtn.onclick = () => openTask(task.id);

  let head = '';

  if (gate) {
    const [label] = kindOf(gate.kind);
    head += `
      <div class="m3-card-attention-box">
        <div style="font-weight:800;font-size:13px;display:flex;align-items:center;gap:6px;">
          <span class="material-symbols-outlined" style="font-size:16px;">pending_actions</span>
          <span>【${esc(label)}】あなたの判断待ち</span>
        </div>
        <div style="font-size:12.5px;">${esc(gate.title)}</div>
        ${stopWhy(gate).length ? `<div style="font-size:11.5px;margin-top:2px;">止めた理由: ${esc(stopWhy(gate).join(' / '))}</div>` : ''}
        <button class="btn-m3-primary" style="margin-top:6px;align-self:flex-start;" data-judge="${esc(gate.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">arrow_forward</span>
          <span>判定画面を開く</span>
        </button>
      </div>
    `;
  }

  let body = '';

  // Newest first: the one the worker left last is the one that describes where it is now.
  const records = recordsOf(task).reverse();
  if (records.length) {
    body += `<div class="m3-filled-card"><div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">記録（止めずに進んだもの）</div>` +
      records.map(r => {
        const [label] = kindOf(r.kind);
        const [text, tone] = recordSummary(r);
        const isGood = tone === 'good';
        const isBad = tone === 'bad';
        const pillClass = isGood ? 'pill-good' : isBad ? 'pill-err' : 'pill-warn';
        const icon = isGood ? 'check_circle' : isBad ? 'cancel' : 'info';
        return `<div class="d-record" style="display:flex;flex-direction:column;gap:4px;padding:8px 0;border-top:1px solid var(--md-sys-color-outline-variant);">
          <div style="display:flex;align-items:center;gap:6px;">
            <span class="m3-pill ${pillClass}">
              <span class="material-symbols-outlined" style="font-size:12px;margin-right:2px;">${icon}</span>
              <span>${esc(label)}: ${esc(text)}</span>
            </span>
          </div>
          <div style="font-size:11px;color:var(--md-sys-color-outline);">${ago(r.openedAt)}に記録</div>
          <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;" data-record="${esc(r.id)}">全体を見る・差し戻す →</button>
        </div>`;
      }).join('') + `</div>`;
  }

  const drawerPrUrl = httpUrl(task.pr);
  body += `
    <div class="m3-filled-card">
      <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">基本情報</div>
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px;font-size:12.5px;">
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">完了条件</span>
          <strong style="color:var(--md-sys-color-on-surface);">${esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')}</strong>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">確認ポイント</span>
          <strong style="color:var(--md-sys-color-on-surface);">${esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt || '—')}</strong>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">ブランチ</span>
          <code style="font-family:var(--font-mono);font-size:12px;color:var(--md-sys-color-on-surface);word-break:break-all;">${esc(task.branch || '—')}</code>
        </div>
        <div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">worktree</span>
          <code style="font-family:var(--font-mono);font-size:12px;color:var(--md-sys-color-on-surface);word-break:break-all;">${esc(task.worktree ? task.worktree.split('/').pop() : '—')}</code>
        </div>
        ${task.executor === 'jules' ? `<div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">実装</span>
          ${httpUrl(task.jules?.url)
            ? `<a href="${esc(task.jules.url)}" target="_blank" rel="noopener noreferrer" style="color:var(--md-sys-color-primary);font-weight:700;text-decoration:none;display:inline-flex;align-items:center;gap:4px;"><span>Jules ${esc(julesText(task.jules))}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>`
            : `<strong style="color:var(--md-sys-color-on-surface);">Jules${task.julesSession ? '' : '（計画の承認後に渡す）'}</strong>`}
        </div>` : ''}
        ${drawerPrUrl ? `<div style="display:flex;flex-direction:column;gap:2px;">
          <span style="color:var(--md-sys-color-outline);font-size:11px;">PR</span>
          <a href="${esc(drawerPrUrl)}" target="_blank" rel="noopener noreferrer" title="${esc(drawerPrUrl)}" style="color:var(--md-sys-color-primary);font-weight:700;text-decoration:none;display:inline-flex;align-items:center;gap:4px;"><span>${prNumberOf(drawerPrUrl) ? `#${esc(prNumberOf(drawerPrUrl))}` : 'PR を開く'}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>
        </div>` : ''}
      </div>
    </div>
  `;

  if (task.worktree) {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:8px;">開発環境の操作</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap;">
          <button class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;" data-focus="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">terminal</span>
            <span>ターミナル前面表示</span>
          </button>
          <button class="btn-m3-tonal" style="padding:6px 14px;font-size:12px;" title="${ideTitle()}" data-ide="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">code</span>
            <span>IDE で開く</span>
          </button>
          ${workerOf(task)?.present ? `<button class="btn-m3-danger" style="padding:6px 14px;font-size:12px;" data-close="${esc(task.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">close</span>
            <span>タブを閉じる</span>
          </button>` : ''}
        </div>
      </div>
    `;
  }

  if (task.julesSession && httpUrl(task.pr) && ['dispatched', 'pr'].includes(task.status)) {
    body += relayHtml(task);
  }

  if (task.instruction && colId !== 'backlog') {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">エージェントへの申し送り（指示）</div>
        <p style="font-size:13px;line-height:1.6;color:var(--md-sys-color-on-surface);white-space:pre-wrap;">${esc(task.instruction)}</p>
      </div>
    `;
  }

  if (task.body) {
    body += `
      <div class="m3-filled-card">
        <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">依頼内容・プロンプト</div>
        <p style="font-size:13px;line-height:1.6;color:var(--md-sys-color-on-surface);white-space:pre-wrap;">${esc(task.body)}</p>
      </div>
    `;
  }

  // The latest few only: the whole history is a click away in the task view's 経過 tab.
  const all = gatesOf(task);
  body += `
    <div class="m3-filled-card" style="display:flex;flex-direction:column;">
      <div style="font-size:11px;font-weight:800;color:var(--md-sys-color-outline);text-transform:uppercase;margin-bottom:6px;">経過</div>
      ${timelineHtml(task, all, 5)}
      <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;align-self:flex-start;margin-top:6px;" data-history="${esc(task.id)}">経過をすべて見る →</button>
    </div>
  `;

  const headEl = document.getElementById('drawer-head');
  const restEl = document.getElementById('drawer-rest');
  if (headEl && restEl) {
    headEl.innerHTML = head;
    restEl.innerHTML = body;
    for (const part of [headEl, restEl]) {
      part.querySelectorAll('[data-record]').forEach(b =>
        b.addEventListener('click', () => openRecord(b.dataset.record)));
      part.querySelectorAll('[data-judge]').forEach(b =>
        b.addEventListener('click', () => judgeGate(b.dataset.judge)));
      // A gate in 経過 opens where the task view reads it, rather than being repeated here.
      part.querySelectorAll('[data-open]').forEach(b => b.addEventListener('click', () => {
        const g = all.find(x => x.id === b.dataset.open);
        if (g) openTask(task.id, TAB_OF_KIND[g.kind] || 'history', g.id);
      }));
      part.querySelectorAll('[data-history]').forEach(b =>
        b.addEventListener('click', () => openTask(b.dataset.history, 'history')));
      part.querySelectorAll('[data-focus]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('focus', b.dataset.focus)));
      part.querySelectorAll('[data-ide]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('ide', b.dataset.ide)));
      part.querySelectorAll('[data-close]').forEach(b =>
        b.addEventListener('click', () => worktreeAct('close', b.dataset.close)));
      part.querySelectorAll('[data-findings]').forEach(b =>
        b.addEventListener('click', () => loadFindings(b.dataset.findings)));
      part.querySelectorAll('[data-relay]').forEach(b =>
        b.addEventListener('click', () => relayPicked(b.dataset.relay)));
      part.querySelectorAll('[data-relay-pick]').forEach(b =>
        b.addEventListener('change', () => {
          if (b.checked) relay.picked.add(b.dataset.relayPick); else relay.picked.delete(b.dataset.relayPick);
          renderDrawer();
        }));
    }
    renderHandForm(colId === 'backlog' ? task : null);
  }
}

/* The hand-over form of a backlog task. The board redraws once a minute and on every change of
   state, and a textarea built again loses what is typed into it, the caret, and an IME
   composition in progress. So the form is built again only for another task, or when the saved
   instruction changed while the box is neither typed in nor focused. When it changed while the
   box is being written, the box is kept and a note says so, with a button to load the new one. */
function renderHandForm(task) {
  const el = document.getElementById('drawer-form');
  if (!el) return;
  if (!task) {
    el.replaceChildren();
    delete el.dataset.task;
    return;
  }
  const saved = task.instruction || '';
  const kept = el.querySelector('#drawer-instruction');
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
        <label for="drawer-instruction" style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);">エージェントへの申し送り（指示）</label>
        <div data-stale hidden style="font-size:11.5px;color:var(--md-sys-color-error);">
          保存済みの申し送りが別の所で変わりました。いまの入力のまま渡すと上書きします。
          <button type="button" class="btn-m3-text" style="padding:2px 6px;font-size:11.5px;" data-reload>変わった内容を読み込む</button>
        </div>
        <textarea id="drawer-instruction" placeholder="追加の指示や申し送りがあれば入力（任意）..." style="width:100%;box-sizing:border-box;border-radius:var(--md-shape-corner-xs);border:1px solid var(--md-sys-color-outline-variant);padding:8px 10px;background:var(--md-sys-color-surface-container-high);color:var(--md-sys-color-on-surface);font-size:12.5px;font-family:inherit;resize:vertical;min-height:60px;">${esc(saved)}</textarea>
        <button class="btn-m3-primary" style="width:100%" data-drawer-hand="${esc(task.id)}">
          <span class="material-symbols-outlined" style="font-size:16px;">send</span>
          <span>待機キューに渡す</span>
        </button>
      </div>
    `;
  const textarea = el.querySelector('#drawer-instruction');
  el.dataset.shown = textarea.value;
  el.querySelector('[data-reload]').addEventListener('click', () => {
    const now = (state.tasks || []).find(t => t.id === task.id);
    if (!now) return;
    el.replaceChildren();
    renderHandForm(now);
  });
  el.querySelector('[data-drawer-hand]').addEventListener('click', () =>
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

document.addEventListener('click', e => {
  if (!e.target.closest('#boards')) return;
  const a = e.target.closest('[data-act]');
  if (a) {
    e.stopPropagation();
    return act(a.dataset.act, a.dataset.id, a.dataset.choice);
  }
  const jh = e.target.closest('[data-jump-human]');
  if (jh) {
    e.stopPropagation();
    return jump('human', jh.dataset.jumpHuman);
  }
  const ja = e.target.closest('[data-jump-agent]');
  if (ja) {
    e.stopPropagation();
    return jump('agent', ja.dataset.jumpAgent);
  }
  const dh = e.target.closest('[data-hand]');
  if (dh) {
    e.stopPropagation();
    return openHandoverDialog(dh.dataset.hand);
  }
  const di = e.target.closest('[data-ide]');
  if (di) {
    e.stopPropagation();
    return worktreeAct('ide', di.dataset.ide);
  }
  const df = e.target.closest('[data-focus]');
  if (df) {
    e.stopPropagation();
    return worktreeAct('focus', df.dataset.focus);
  }
  const dg = e.target.closest('[data-gate]');
  if (dg) {
    e.stopPropagation();
    return judgeGate(dg.dataset.gate);
  }
  const dr = e.target.closest('[data-record]');
  if (dr) {
    e.stopPropagation();
    return openRecord(dr.dataset.record);
  }
});

document.addEventListener('keydown', e => {
  if (e.target.matches('textarea[data-reply]')) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      const id = e.target.dataset.reply;
      const open = openReplies[id];
      if (open) act(`send-${open}`, id);
    }
  }
});

document.getElementById('boards')?.addEventListener('focusout', e => {
  if (!boardHeld || !e.target.matches('textarea[data-reply]')) return;
  setTimeout(() => {
    if (!boardHeld || document.activeElement?.matches('textarea[data-reply]')) return;
    renderColumns();
  });
});



