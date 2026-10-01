/* What `adj phase --set` writes, in the words a card shows. */
const PHASE_LABEL = { plan:'計画', implement:'実装', 'self-review':'セルフレビュー', verify:'動作確認',
                      pr:'PR', 'pr-bots':'bot待ち', review:'レビュー対応', report:'報告' };
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
  // Once its PR is open the card waits on reviewers or bots, and a worker whose tab was
  // closed then has simply finished — flagging those would bury the ones that are stuck.
  if (task.status === 'pr') return null;
  const w = workerOf(task);
  // The worker is gone and nothing will move this card: the one thing a person must hear.
  // Not in the first two minutes, while a worker that was just dispatched is still opening.
  if (!w || !w.present) return Date.now() - updatedMs(task) < 120000 ? null : 'worker 停止';
  // Time only counts while the ball is the worker's. A card waiting on a person's answer to a
  // gate is not the worker being stuck.
  if (openGate(task)) return null;
  const mins = workerMinutes(task, w);
  const limit = state.stuckAfterMinutes;
  if (mins != null && limit > 0 && mins >= limit) return `${minutesLabel(mins)} 同じ工程`;
  return null;
}

/* What a card says about the Jules session behind it, in the words the card shows. `working`
   comes from the server, which knows which states mean Jules is busy. */
const JULES_LABEL = { QUEUED:'待機中', PLANNING:'計画中', IN_PROGRESS:'作業中', AWAITING_PLAN_APPROVAL:'計画の承認待ち',
                      AWAITING_USER_FEEDBACK:'返事待ち', PAUSED:'一時停止', COMPLETED:'完了', FAILED:'失敗' };
/* The one rule for what a session's state reads as, for the card, the task panel and the full
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
async function focusHub(slug = null) {
  const line = 'adj focus';
  try {
    const data = await boardApi(slug ? `/b/${slug}` : BASE, '/api/hub/focus', { method: 'POST', body: '{}' });
    note(line, false, data.present ? (data.ran ? 'hub のタブを前に出しました' : 'hub のタブが見つかりませんでした') : 'hub は動いていません');
  } catch (e) { note(`${line} → ${e.message}`, true); }
}

/* `confirmed` is set by the close dialog: closing stops the worker, so it is asked there first. */
/* The board a worktree belongs to, for the merged state of 「すべて」 and the review queue. */
const slugOfWorktree = wt => ((state.gates || []).find(g => g.worktree === wt) || (state.tasks || []).find(t => t.worktree === wt))?._slug || null;

async function worktreeAct(action, worktree, confirmed = false, slug = null) {
  if (!slug && scopeAll()) slug = slugOfWorktree(worktree);
  const line = { focus: `adj focus --worktree ${worktree}`, ide: `adj ide --worktree ${worktree}`,
                 close: `adj close --worktree ${worktree}` }[action];
  // With no editor configured the server can only refuse, so say how to set one instead.
  if (action === 'ide' && !ideReady()) { openIdeDialog(); return; }
  if (action === 'close' && !confirmed) { openCloseDialog(worktree); return; }
  try {
    const data = await boardApi(slug ? `/b/${slug}` : BASE, `/api/worktrees/${action}`, { method: 'POST', body: JSON.stringify({ worktree }) });
    const why = action === 'focus' ? (data.present ? (data.ran ? 'タブを前に出しました' : 'worker のタブが見つかりませんでした') : 'worker は動いていません')
              : action === 'close' ? (data.closed ? 'タブを閉じました' : 'まだ閉じていません（確認待ちかもしれません）')
              : 'エディタで開きました';
    note(line, false, why);
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
    if (view === 'sessions') showSessNotice(`${line}: ${e.message}`, true);
  }
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
// Keyed like the review queue names a record, so records of several boards do not share a mark.
const isUnread = r => !seenRecords().has(gateRef(r));
function markSeen(id) {
  seenCache = null;
  const seen = seenRecords();
  if (seen.has(id)) return;
  seen.add(id);
  const live = new Set(allRecords().map(gateRef));
  try { localStorage.setItem(SEEN_KEY(), JSON.stringify([...seen].filter(x => live.has(x)))); }
  catch {}
}
window.addEventListener('storage', e => {
  if (e.key !== SEEN_KEY()) return;
  seenCache = null;
  if (view === 'board') render();
  else if (view === 'task') redrawTaskView();
});
// A record of a merged state carries its board, so answering it reaches the right one.
const allRecords = () => (state.tasks || []).flatMap(t => t._base ? (t.records || []).map(r => ({ ...r, _slug: t._slug, _base: t._base })) : t.records || []);
const recordById = id => allRecords().find(r => r.id === id);
const recordByRef = ref => allRecords().find(r => gateRef(r) === ref) || (ref && !String(ref).includes('/') ? recordById(ref) : undefined);
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
    chips.push({ id: r.id, text: text + ((r.answers || []).length ? ' ↩' : ''), tone, unread: isUnread(task._slug && !r._slug ? { ...r, _slug: task._slug } : r) });
    if (r.kind === 'verify' && (r.manual || []).length) {
      chips.push({ id: r.id, text: `手で見る ${r.manual.length}件`, tone: '', unread: false });
    }
  }
  return chips;
}

function openRecord(id) {
  const r = recordById(id);
  if (r && r.task) return openTask(r.task, TAB_OF_KIND[r.kind] || 'history', id);
  goToGate(id);
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
    const gate = t ? openGate(t) : (state.gates || []).find(g => gateRef(g) === id);
    const replyEl = document.querySelector(`textarea[data-reply="${id}"]`);
    const reply = replyEl ? replyEl.value.trim() : '';

    switch (action) {
      case 'start': {
        if (gate) {
          await submitAnswer('approve', undefined, gateRef(gate), '');
        } else if (t) {
          try {
            await boardApi(baseOf(t), `/api/tasks/${encodeURIComponent(t.id)}`, {
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
          await submitAnswer('reject', undefined, gateRef(gate), '');
        } else if (t) {
          // The queue is one board's; from 「すべて」 the card's own board takes it.
          if (scopeAll()) {
            try {
              await boardApi(baseOf(t), `/api/tasks/${encodeURIComponent(t.id)}`, {
                method: 'POST', body: JSON.stringify({ status: 'backlog' }),
              });
            } catch (e) {
              note(`adj task update --id ${t.id} --status backlog → ${e.message}`, true);
              succeeded = false;
            }
          } else await move(t.id, 'backlog');
        }
        break;
      }
      case 'approve': {
        const decision = gate?.kind === 'result' ? 'ack' : 'approve';
        if (gate) {
          await submitAnswer(decision, undefined, gateRef(gate), reply);
        }
        break;
      }
      case 'choice': {
        if (gate && choice) {
          await submitAnswer('choice', choice, gateRef(gate), reply);
        }
        break;
      }
      case 'send-reject': {
        const decision = gate?.kind === 'verify' || gate?.kind === 'diff' ? 'changes' : 'reject';
        if (gate) {
          await submitAnswer(decision, undefined, gateRef(gate), reply || '差し戻し');
        }
        break;
      }
      case 'send-changes': {
        if (gate) {
          await submitAnswer('changes', undefined, gateRef(gate), reply || '修正指示');
        } else if (t && reply) {
          try {
            await boardApi(baseOf(t), `/api/tasks/${encodeURIComponent(t.id)}`, {
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
          await submitAnswer('answer', undefined, gateRef(gate), reply || '回答');
        }
        break;
      }
      case 'send-ask': {
        if (gate) {
          await submitAnswer('ask', undefined, gateRef(gate), reply || '追加の質問');
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

/* In 「すべて」 a card says which board it came from; on a board of its own that is known. */
function originChip(item) {
  if (!scopeAll() || !item?._slug) return '';
  const b = boards.find(x => x.slug === item._slug);
  const name = b ? (b.hub || repoNameOf(b)) : item._slug;
  const title = b ? boardName(b) + (b.hub ? ` (${b.nwo})` : '') : item._slug;
  return `<span class="origin-chip" title="${esc(title)}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(name)}</span></span>`;
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
      ${scopeAll() ? '' : `<button type="button" class="btn-m3-tonal" data-act="refresh-prs" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">sync</span><span>PR確認</span>
      </button>`}
      <button type="button" class="btn-m3-text" data-act="changes" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">rate_review</span><span>指摘をメモ</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(task.id)}">
        <span class="material-symbols-outlined">chat</span><span>回答する</span>
      </button>
      ${readySessionOfTask(task) ? `<button type="button" class="btn-m3-tonal" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}">
        <span class="material-symbols-outlined">terminal</span><span>ターミナルで答える</span>
      </button>` : ''}
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
  const open = openReplies[gateRef(gate)];
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
        <textarea data-reply="${esc(gateRef(gate))}" placeholder="${esc(ph)}"></textarea>
        <div class="hcard-actions">
          <button type="button" class="btn-m3-primary" data-act="send-${open}" data-id="${esc(gateRef(gate))}">
            <span class="material-symbols-outlined">send</span>
            <span>${esc(sendLabel)}</span>
          </button>
          <button type="button" class="btn-m3-text" data-act="cancel" data-id="${esc(gateRef(gate))}">やめる</button>
        </div>
      </div>
    `;
  }
  let buttons = '';
  if (col === 'dispatch') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">play_arrow</span><span>着手する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">undo</span><span>Backlog に戻す</span>
      </button>
    `;
  } else if (col === 'plan' || col === 'diff') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">check</span><span>承認する</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="reject" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">reply</span><span>差し戻す</span>
      </button>
    `;
  } else if (col === 'verify') {
    const isResult = gate.kind === 'result';
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="approve" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">check</span><span>${isResult ? '了解' : '確認した'}</span>
      </button>
      <button type="button" class="btn-m3-tonal" data-act="${isResult ? 'ask' : 'reject'}" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">${isResult ? 'help' : 'reply'}</span><span>${isResult ? '追加で聞く' : '直してほしい'}</span>
      </button>
    `;
  } else if (col === 'question') {
    buttons = `
      <button type="button" class="btn-m3-primary" data-act="answer" data-id="${esc(gateRef(gate))}">
        <span class="material-symbols-outlined">chat</span><span>回答する</span>
      </button>
    `;
  }
  if (gate.choices && gate.choices.length) {
    buttons += gate.choices.map(c => `
      <button type="button" class="btn-m3-tonal" data-act="choice" data-choice="${esc(c.id)}" data-id="${esc(gateRef(gate))}">
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
  if (task._slug) el.dataset.slug = task._slug;

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
    ${originChip(task)}
    <div class="title">${esc(task.title)}</div>
    ${col === 'question' ? `<div class="question-box">${esc(why)}</div>` : (why ? `<div class="why">${esc(why)}</div>` : '')}
    ${col === 'prreview' && prUrl ? `<div class="pills"><a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" onclick="event.stopPropagation()" class="m3-pill pill-blue" style="text-decoration:none;" title="PRを開く"><span class="material-symbols-outlined">merge</span>PR #${esc(prNumber || '')}</a></div>` : ''}
    ${humanActions(task, col, gate)}
    <div class="hcard-foot">
      <span>${esc(phaseStr)}</span>
      <span class="hcard-foot-links">
        ${readySessionOfTask(task) ? `<button type="button" class="agent-back" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}">
          <span class="material-symbols-outlined">terminal</span>
          <span>ターミナル</span>
        </button>` : ''}
        <button type="button" class="agent-back" data-jump-agent="${esc(task.id)}" title="エージェントのボードでこのカードを見る">
          <span class="material-symbols-outlined">smart_toy</span>
          <span>エージェントで見る</span>
        </button>
      </span>
    </div>
  `;

  el.onclick = (e) => {
    if (e.target.closest('button') || e.target.closest('a') || e.target.closest('textarea') || e.target.closest('input')) return;
    onBoard(task._slug, () => openTaskPanel(task.id));
  };
  return el;
}

function humanGateCard(gate, col) {
  const el = document.createElement('div');
  el.className = 'card hcard';
  el.id = `human-${gate.id}`;
  el.dataset.id = gateRef(gate);
  if (gate._slug) el.dataset.slug = gate._slug;

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
    ${originChip(gate)}
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
    onBoard(gate._slug, () => judgeGate(gate.id));
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
  if (task._slug) el.dataset.slug = task._slug;

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
  h += originChip(task);
  h += `<div class="title">${esc(task.title)}</div>`;

  // 3. Worker Status (or Jules)
  if (live && task.jules) {
    h += julesLine(task);
  } else if (worker && worker.phase) {
    const mins = phaseMinutes(worker);
    const stopped = !worker.present;
    // A worker gone once its PR is open has finished rather than stopped (see stuckOf).
    const alarm = stopped && !hcol && task.status !== 'pr';
    h += `
      <div class="card-worker-status">
        ${hcol ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-warning);">pause_circle</span>'
               : alarm ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-error);">pause</span>'
               : stopped ? '<span class="material-symbols-outlined" style="font-size:14px;color:var(--md-sys-color-outline);">pause</span>'
               : '<span class="pulse-dot"></span>'}
        <span style="font-weight:700;${alarm ? 'color:var(--md-sys-color-error);' : ''}">${esc(PHASE_LABEL[worker.phase] || worker.phase)}${alarm ? ' (停止)' : ''}</span>
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
          ${readySessionOfTask(task) ? `<button type="button" class="m3-icon-button" title="内蔵ターミナルをパネルで開く" data-term-session="${esc(task.id)}"><span class="material-symbols-outlined" style="font-size:14px;">terminal</span><span>ターミナル</span></button>` : ''}
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
    onBoard(task._slug, () => openTaskPanel(task.id));
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
            ${def.id === 'prreview' && !scopeAll() ? `<button type="button" class="col-btn-nudge" title="PRマージ済みタスクを確認" data-act="refresh-prs"><span class="material-symbols-outlined" style="font-size:13px;">sync</span><span>PR確認</span></button>` : ''}
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

      const nextBtn = def.id === 'before' && !scopeAll()
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

/* A gate is named by its board and id in the review queue, which reads several boards: two of
   them can open a gate of one kind in the same second. `slug` is the board it is on; from a
   board's own page that is the board shown. */
function goToGate(gateId, slug = null) {
  const on = slug || (multiBoard && nav.board && nav.board !== 'all' ? nav.board : null);
  const ref = on ? `${on}/${gateId}` : gateId;
  focused = ref;
  go({ view: 'review', item: ref }, { replace: view === 'review' });
  renderReview();
}

/* Where a waiting gate is read and answered: its task's view, in the tab of its kind, when the
   task is on the board. A gate with no task — the hub's, or one whose task is gone — has no
   such view and opens in the review view. */
function judgeGate(gateId) {
  const g = (state.gates || []).find(x => x.id === gateId);
  const owner = g?.task && (state.tasks || []).find(t => t.id === g.task && (!g._slug || t._slug === g._slug));
  if (owner) onBoard(g._slug, () => openTask(owner.id, TAB_OF_KIND[g.kind] || 'history', g.id));
  else goToGate(gateId);
}

/* The レビュー tab: the first gate in the queue, where it is answered; the review view when the
   queue is empty, which says so. With several boards the queue is the oldest gate of any of them. */
function goToQueue() {
  // Already on the queue: stay on the one being read, and on whatever is typed for it.
  if (view === 'review') return;
  if (multiBoard) {
    const first = everyGate().sort((a, b) => (a.openedAt || '').localeCompare(b.openedAt || ''))[0];
    if (first) return onBoard(first._slug, () => judgeGate(first.id));
    return go({ view: 'review' });
  }
  const first = (state.gates || [])[0];
  if (first) judgeGate(first.id);
  else setView('review');
}

/* The review comments of the task the panel is open on, once a person asked for them.
   Kept here rather than in /api/state: listing them is a round trip to GitHub, done when
   somebody wants to choose, and the choice has to survive the panel being redrawn. */
let relay = { taskId: null, loading: false, error: '', findings: [], picked: new Set() };

async function loadFindings(id) {
  const asked = { taskId: id, loading: true, error: '', findings: [], picked: new Set() };
  relay = asked;
  renderTaskPanel();
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
  renderTaskPanel();
}

async function relayPicked(id) {
  const comments = [...relay.picked];
  // One at a time: a second click while the first is posting would only come back refused.
  if (!comments.length || relay.posting) return;
  const asked = relay;
  asked.posting = true;
  renderTaskPanel();
  const line = `adj jules relay --id ${id} ${comments.map(c => `--comment ${c}`).join(' ')}`;
  try {
    await api(`/api/tasks/${encodeURIComponent(id)}/relay`, { method: 'POST', body: JSON.stringify({ comments }) });
    note(line, false, `${comments.length} 件を PR にコメントしました。Jules が読んで直します`);
    // Read again only if the panel is still on this list; otherwise the reload would
    // replace whatever is being looked at now.
    if (relay === asked && selectedTaskId === id) await loadFindings(id);
  } catch (e) { note(`${line} → ${e.message}`, true); }
  asked.posting = false;
  if (relay === asked) renderTaskPanel();
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
  selectedTaskId = id;
  markSelectedCards();
  renderTaskPanel();
}

/* A click on a card or one of its buttons: another card is a step in the history, the other tab
   of the card that is open replaces the one it is on. */
function openTaskPanel(id, pane = 'detail') {
  pendingTask = null;
  selectedTaskId = id;
  markSelectedCards();
  go({ task: id, pane }, { replace: id === nav.task });
}

/* The panel's state, without the address: for a move the address already made. */
function hideTaskPanelState() {
  pendingTask = null;
  panelPop = false;
  selectedTaskId = null;
  panelScrolledFor = null;
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

/* 'left' and 'right' are the saved side; 'pop' is a large dialog that goes back to the side
   when closed or clicked away from. */
function placePanel(where) {
  if (where === 'pop') panelPop = true;
  else {
    panelPop = false;
    prefs.panelSide = where;
    savePrefs();
  }
  renderTaskPanel();
}

/* A session is there to open a terminal on when its worktree has one at all. */
const hasSession = s => !!s && sessionState(s) !== 'none';
/* The tab shown: ターミナル only where there is a session, whatever the address says. */
const paneOf = task => nav.pane === 'term' && hasSession(sessionOfTask(task)) ? 'term' : 'detail';

/* The sidebar is the icon rail when the window is narrow, when a session's terminal takes the
   width of the セッション tab, and while the panel sits on its left. */
const narrowRail = matchMedia('(max-width: 1024px)');
const termRail = matchMedia('(max-width: 1199px)');
function applyRailMode() {
  const cls = document.body.classList;
  const byPanel = cls.contains('panel-open') && !cls.contains('panel-right') && !cls.contains('panel-pop');
  cls.toggle('rail-icons', narrowRail.matches || byPanel || (cls.contains('sess-term-open') && termRail.matches));
}
narrowRail.addEventListener('change', applyRailMode);
termRail.addEventListener('change', applyRailMode);
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
  const task = selectedTaskId ? taskById(selectedTaskId) : null;
  // A card the board no longer lists takes the panel with it.
  if (selectedTaskId && !task) return dismissTaskPanel();
  const shown = !!task && (view === 'board' || view === 'sessions');
  const cls = document.body.classList;
  panel.hidden = !shown;
  tp('tp-scrim').hidden = !(shown && panelPop);
  cls.toggle('panel-open', shown);
  cls.toggle('panel-right', prefs.panelSide === 'right');
  cls.toggle('panel-pop', shown && panelPop);
  document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
  applyRailMode();
  // The list marks the session the panel is open on.
  if (view === 'sessions') applySessionSelection();
  // Closed, not just out of view: what was typed for the task goes with it.
  if (!task) return renderHandForm(null);
  // In the task view the panel waits, with its terminal, for the board to come back.
  if (!shown) return;

  const colId = columnOf(task);
  const gate = openGate(task);
  const s = sessionOfTask(task);
  const pane = paneOf(task);

  setPanelPart('head', tp('tp-head'), panelHeadHtml(task));
  setPanelPart('tabs', tp('tp-tabs'), panelTabsHtml(task, gate, s, pane));

  // Shown before the terminal is mounted: a hidden host has no size to fit to.
  const reveal = pane === 'term' && tp('tp-term').hidden;
  tp('tp-detail').hidden = pane === 'term';
  tp('tp-term').hidden = pane !== 'term';
  // Another task starts at the top, not where the last one was scrolled to: set once the pane
  // is shown, since a hidden one has no scroll to set.
  if (pane === 'detail' && panelScrolledFor !== task.id) {
    panelScrolledFor = task.id;
    tp('tp-detail').scrollTop = 0;
  }
  syncPanelTerminal(task, s, pane);
  setPanelPart('bar', tp('tp-term-bar'), termBarHtml(s));
  const ph = tp('tp-term-ph');
  ph.hidden = !!panelTerm.term;
  setPanelPart('ph', ph, panelTerm.term ? '' : termPlaceholderHtml(s));
  if (reveal && panelTerm.term) {
    // It was sized while hidden, which it skips; asked again now that it has a size.
    panelTerm.term.fit();
    panelTerm.term.focus();
  }

  setPanelPart('gate', tp('tp-gate'), panelGateHtml(task, gate));
  setPanelPart('rest', tp('tp-rest'), panelRestHtml(task, colId));
  renderHandForm(colId === 'backlog' ? task : null);
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
  const place = (where, icon, label, on) =>
    `<button type="button" class="tool-btn${on ? ' on' : ''}" data-tp-place="${where}" title="${label}" aria-label="${label}" aria-pressed="${on}"><span class="material-symbols-outlined" aria-hidden="true">${icon}</span></button>`;
  return `
    <div class="tp-head-main">
      <div class="tp-badges">
        <span class="tp-key" title="${esc(task.id)}">${esc(task.id)}</span>
        ${origin ? `<span class="origin-chip" title="${esc(b ? `${boardName(b)} (${b.nwo})` : state.repo || '')}"><span class="material-symbols-outlined" aria-hidden="true">${b?.hub ? 'account_tree' : 'folder'}</span><span>${esc(origin)}</span></span>` : ''}
        ${pill}
      </div>
      <h2 class="tp-title">${esc(task.title)}</h2>
    </div>
    <div class="tp-head-btns">
      <button type="button" class="btn-m3-text tp-jump" data-tp-jump title="エージェントのボードでこのカードを見る">カードへ</button>
      ${place('left', 'left_panel_open', '左に置く', prefs.panelSide === 'left' && !panelPop)}
      ${place('right', 'right_panel_open', '右に置く', prefs.panelSide === 'right' && !panelPop)}
      ${place('pop', 'open_in_new', 'ポップアウト', panelPop)}
      <button type="button" class="tool-btn" data-tp-close title="閉じる" aria-label="閉じる"><span class="material-symbols-outlined" aria-hidden="true">close</span></button>
    </div>`;
}

function panelTabsHtml(task, gate, s, pane) {
  const usable = hasSession(s) && !!state.boardTerminal?.available;
  const hint = hasSession(s) ? '端末はボードから開けません' : 'セッションなし';
  const tab = (id, label, extra, disabled) =>
    `<button type="button" role="tab" id="tp-tab-${id}" class="tp-tab${pane === id ? ' on' : ''}" data-pane="${id}" aria-selected="${pane === id}" aria-controls="${id === 'term' ? 'tp-term' : 'tp-detail'}"${disabled ? ` disabled title="${hint}"` : ''}>${label}${extra}</button>`;
  return tab('detail', '詳細', gate ? '<span class="tp-dot" title="あなたの判断待ちがあります"></span>' : '', false)
    + tab('term', 'ターミナル', !usable ? `<span class="tp-tab-hint">${hint}</span>` : s?.waiting ? '<span class="tp-wait">入力待ち</span>' : '', !usable);
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
  const col = gateHumanCol(gate.kind);
  const why = gate.problem || gate.why || '';
  const reasons = stopWhy(gate);
  const quick = col === 'dispatch' ? [['start', '着手する', 'play_arrow']]
    : col === 'plan' ? [['approve', '承認', 'check']]
    : col === 'diff' ? [['approve', '承認して PR へ', 'check']]
    : col === 'verify' ? [['approve', gate.kind === 'result' ? '了解' : '確認した', 'check']]
    : [];
  const btn = ([action, text, icon]) =>
    `<button type="button" class="btn-m3-primary" data-tp-act="${action}"><span class="material-symbols-outlined" style="font-size:16px;">${icon}</span><span>${text}</span></button>`;
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
    ${worker?.phase ? `<div class="tp-line">worker は${esc(PHASE_LABEL[worker.phase] || worker.phase)}${worker.present ? '' : '（停止）'}${mins != null ? `（${esc(minutesLabel(mins))}前から）` : ''}</div>` : ''}
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
  const prUrl = httpUrl(task.pr);
  h += `<div class="m3-filled-card">${secTitle('記録（止めずに進んだもの）')}
    ${prUrl ? `<div class="tp-kvs">${kv('PR', `<a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" title="${esc(prUrl)}" class="tp-link"><span>${prNumberOf(prUrl) ? `#${esc(prNumberOf(prUrl))}` : 'PR を開く'}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>`)}</div>` : ''}
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
    }).join('') : (prUrl ? '' : '<div class="tp-muted">記録はまだありません</div>')}
  </div>`;
  if (task.julesSession && prUrl && live) h += relayHtml(task);

  h += `<div class="m3-filled-card">${secTitle('作業場所')}
    ${task.worktree || task.branch ? `<div class="tp-kvs">
      ${monoKv('worktree', task.worktree ? task.worktree.split('/').pop() : '—', task.worktree || '')}
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

/* ── ターミナル ── */
/* The terminal lives in #tp-term-host from its first mount until the task changes or the panel
   closes. 詳細 only hides the pane around it, so the socket survives; the bar and the note
   over it are the parts that are drawn again. */
function syncPanelTerminal(task, s, pane) {
  // Another task's socket is not carried over; a fresh one is asked for after 再開 or 再接続.
  if (panelTerm.taskId !== task.id || (panelTerm.term && s && s.id !== panelTerm.sessionId)
      || (panelTerm.reconnect && boardTerminalReady(s))) disposePanelTerminal();
  panelTerm.taskId = task.id;
  if (pane !== 'term' || panelTerm.term || !boardTerminalReady(s)) return;
  // One session, one terminal: the セッション tab lets go of it.
  const heldBySessions = sessView.selectedId === s.id;
  if (heldBySessions) detachSessionTerminal();
  panelTerm.sessionId = s.id;
  panelTerm.ended = null;
  const handle = mountSessionTerminal(tp('tp-term-host'), {
    sessionId: s.id,
    onEnd: code => {
      if (panelTerm.term !== handle) return;
      panelTerm.ended = code;
      keepScreen({ sessionId: s.id, term: handle });
      renderTaskPanel();
    },
  });
  panelTerm.term = handle;
  // The sessions tab says where its terminal went.
  if (heldBySessions && view === 'sessions') renderSessionsView();
}

function disposePanelTerminal() {
  const { term, sessionId } = panelTerm;
  if (term) {
    keepScreen({ sessionId, term });
    term.dispose();
  }
  Object.assign(panelTerm, { taskId: null, sessionId: null, term: null, ended: null, reconnect: false });
  // The sessions tab may mount the session again, now that the panel has let go of it.
  if (term && view === 'sessions') renderSessionsView();
}

function termBarHtml(s) {
  if (!s || !hasSession(s)) return '';
  const st = sessionState(s);
  const last = s.present ? lastOutputText(s) : null;
  const again = panelTerm.term && panelTerm.ended != null && boardTerminalReady(s);
  return `<span class="m3-pill ${STATE_PILL[st] || 'pill-neutral'}">${esc(STATE_LABEL[st])}</span>`
    + (last ? `<span class="tp-muted">最後の出力: ${esc(last)}</span>` : '')
    + `<span class="tp-bar-gap"></span>`
    + (again ? '<button type="button" class="btn-m3-tonal sess-act" data-tp-reconnect><span class="material-symbols-outlined" aria-hidden="true">sync</span><span>再接続</span></button>' : '')
    + sessionButtons(s).bar.map(b => actionButtonHtml(b)).join('');
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
  const mins = shot && state.now != null && shot.at != null ? Math.max(0, Math.floor((state.now - shot.at) / 60)) : null;
  const resume = canResume(s) ? '' : !s.present && s.kind === 'worker'
    ? `<div class="tp-muted">${esc(!s.conversation ? '保存された会話がないため再開できません' : state.sessionResume?.reason || 'ボードからは再開できません')}</div>` : '';
  return `<div class="tp-ph-head">${esc(head)}</div>`
    + (shot?.lines.length ? `<div class="tp-muted">このページで最後に見た画面${mins == null ? '' : `（${esc(minutesLabel(mins))}前）`}</div><pre class="sess-last-out">${esc(shot.lines.join('\n'))}</pre>` : '')
    + resume;
}

/* Events are heard on the panel itself: its parts are drawn again, it is not. */
tp('task-panel').addEventListener('click', e => {
  const task = taskById(selectedTaskId);
  if (!task) return;
  const hit = sel => e.target.closest(sel);
  let b;
  if ((b = hit('[data-pane]'))) { if (!b.disabled) go({ pane: b.dataset.pane }, { replace: true }); return; }
  if (hit('[data-tp-close]')) {
    // Closing a popped-out panel puts it back on its side, as a click outside does; the next closes it.
    if (panelPop) { panelPop = false; return renderTaskPanel(); }
    return closeTaskPanel();
  }
  if ((b = hit('[data-tp-place]'))) return placePanel(b.dataset.tpPlace);
  if (hit('[data-tp-jump]')) {
    // A popped-out panel is over the card: it goes back to its side first.
    panelPop = false;
    renderTaskPanel();
    return jump('agent', task.id);
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
tp('task-panel').addEventListener('change', e => {
  const b = e.target.closest('[data-relay-pick]');
  if (!b) return;
  if (b.checked) relay.picked.add(b.dataset.relayPick); else relay.picked.delete(b.dataset.relayPick);
  renderTaskPanel();
});
// Clicking outside a popped-out panel puts it back; it stays open.
tp('tp-scrim').addEventListener('click', () => { panelPop = false; renderTaskPanel(); });

/* The width is dragged from the edge that faces the page, saved when the pointer is let go. */
tp('tp-resize').addEventListener('pointerdown', e => {
  if (panelPop || matchMedia('(max-width: 720px)').matches) return;
  e.preventDefault();
  const handle = e.currentTarget;
  handle.setPointerCapture(e.pointerId);
  document.body.classList.add('tp-dragging');
  const move = ev => {
    const rail = tp('nav-rail').offsetWidth;
    const raw = prefs.panelSide === 'right' ? innerWidth - ev.clientX : ev.clientX - rail;
    prefs.panelWidth = Math.round(Math.max(320, Math.min(raw, innerWidth - rail - 320)));
    document.body.style.setProperty('--panel-w', `${prefs.panelWidth}px`);
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

/* The board a card came from, which only 「すべて」 draws cards of. */
const slugOf = el => scopeAll() ? el.closest('[data-slug]')?.dataset.slug || null : null;

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
    return onBoard(slugOf(dh), () => openHandoverDialog(dh.dataset.hand));
  }
  const di = e.target.closest('[data-ide]');
  if (di) {
    e.stopPropagation();
    return worktreeAct('ide', di.dataset.ide, false, slugOf(di));
  }
  const dt = e.target.closest('[data-term-session]');
  if (dt) {
    e.stopPropagation();
    return onBoard(slugOf(dt), () => openTaskPanel(dt.dataset.termSession, 'term'));
  }
  const dg = e.target.closest('[data-gate]');
  if (dg) {
    e.stopPropagation();
    return onBoard(slugOf(dg), () => judgeGate(dg.dataset.gate));
  }
  const dr = e.target.closest('[data-record]');
  if (dr) {
    e.stopPropagation();
    return onBoard(slugOf(dr), () => openRecord(dr.dataset.record));
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



