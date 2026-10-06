/* The decision a board button's `data-act` ends up sending: act() remaps these by kind. */
const sentDecision = (action, kind) =>
  action === 'approve' ? (kind === 'result' ? 'ack' : 'approve')
  : action === 'start' ? 'approve'
  : action === 'shelve' ? 'reject'
  : action === 'reject' && (kind === 'plan' || kind === 'verify' || kind === 'diff') ? 'changes'
  : action;
const actLabel = (action, kind) => decisionLabel(sentDecision(action, kind), kind);

/* Minutes since the worker entered its phase, by the server's clock so a laptop that slept
   does not make every card look stuck. */
const phaseMinutes = w => w && w.phaseAt != null && state.now != null
  ? minutesSince(w.phaseAt, state.now) : null;

/* Minutes the worker has had the ball: since it entered its phase, or since a person last
   answered its gate, whichever is later. */
const workerMinutes = (task, w) => {
  const mins = phaseMinutes(w);
  const answered = stampSecs(task.gateAnsweredAt);
  if (mins == null || answered == null || state.now == null) return mins;
  return Math.min(mins, minutesSince(answered, state.now));
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
    const mins = minutesSince(since / 1000, Date.now() / 1000);
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
      ${url ? `<a href="${esc(url)}" target="_blank" rel="noopener noreferrer" style="color:inherit;text-decoration:none;">${label}</a>` : label}
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
  // Closing under a restart would race it: the restart closes this window itself.
  if (action === 'close') {
    const restarting = (state.sessions || []).find(x => x.kind === 'worker' && x.worktree === worktree && restartingNow(x));
    if (restarting) return note(line, true, '再起動しています。終わってから操作してください');
  }
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
        const decision = sentDecision('reject', gate?.kind);
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
      const gatesInCol = standaloneGates.filter(g => g.humanCol === def.id);
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
            ${def.id === 'prreview' && !scopeAll() ? `<button type="button" class="col-btn-nudge" title="PR の状態を読み直す" data-act="refresh-prs"><span class="material-symbols-outlined" style="font-size:13px;">sync</span><span>PR確認</span></button>` : ''}
          </div>
          ${def.hint ? `<div class="col-subtext" title="${esc(def.hint)}">${esc(def.hint)}</div>` : ''}
          ${def.id === 'prreview' && state.prPoll?.error ? `<div class="col-warn" title="${esc(state.prPoll.error)}">PR の自動確認が止まっています: ${esc(state.prPoll.error)}</div>` : ''}
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
    // A session with no task has no phase to place it by, so it sits with the work that is
    // under way; a column that does not draw these cards sends them to 実装 so none vanishes.
    const known = new Set((state.tasks || []).map(t => t.worktree).filter(Boolean));
    // A worker that names a task of the board belongs to that task's card, which finds its session by `s.task`.
    const ofBoard = w => w.task && (state.tasks || []).some(t => t.id === w.task && (!w._slug || t._slug === w._slug));
    const bare = (state.workers || []).filter(w => w.present && !known.has(w.worktree) && !ofBoard(w));
    const bareCol = w => {
      const c = AGENT_COL_OF_PHASE[w.phase];
      return c && c !== 'before' && c !== 'done' ? c : 'implement';
    };
    for (const def of AGENT_COLUMNS) {
      const bareIn = def.id === 'before' || def.id === 'done' ? [] : bare.filter(w => bareCol(w) === def.id);
      const items = (state.tasks || []).filter(t => agentColOf(t) === def.id);
      const isNarrow = def.id === 'before' || def.id === 'done';
      let older = [];
      let displayItems = items;
      if (def.id === 'done') {
        const cutoff = Date.now() - DONE_SHOWN_HOURS * 3600 * 1000;
        older = items.filter(t => updatedMs(t) < cutoff);
        if (!showOlderDone) displayItems = items.filter(t => updatedMs(t) >= cutoff);
      }

      const totalCount = (def.id === 'done' ? (showOlderDone ? items.length : displayItems.length) : items.length) + bareIn.length;
      const isEmpty = totalCount === 0;
      const col = document.createElement('section');
      col.className = 'col' + (isNarrow ? ' narrow' : '') + (isEmpty ? ' empty' : '');
      col.dataset.col = def.id;

      const nextBtn = def.id === 'before' && !scopeAll()
        ? '<button type="button" class="col-btn-nudge" title="workerの枠が空いていれば次を着手" data-action="nudge-hub"><span class="material-symbols-outlined" style="font-size:13px;">bolt</span><span>次を流す</span></button>'
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
        if (!displayItems.length && !bareIn.length) {
          cards.insertAdjacentHTML('beforeend', '<div class="col-empty-placeholder">なし</div>');
        } else {
          for (const t of displayItems) cards.appendChild(agentCard(t));
        }
        for (const w of bareIn) cards.appendChild(sessionCard(w));

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

/* The レビュー tab: the review view, one queue across every board. Without an item it picks the
   first one waiting on its own. */
function goToQueue() {
  // Already on the queue: stay on the one being read, and on whatever is typed for it.
  if (view === 'review') return;
  // Not the item last read in another view: the queue's first is.
  focused = null;
  reviewPane = 'judge';
  if (multiBoard) return go({ view: 'review' });
  setView('review');
  renderReview();
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
  const so = e.target.closest('[data-sess-open]');
  if (so) {
    e.stopPropagation();
    return onBoard(slugOf(so), () => openSessionRef(so.dataset.sessOpen));
  }
  const sl = e.target.closest('[data-sess-link]');
  if (sl) {
    e.stopPropagation();
    // The dialog posts to the page's board, so it opens once the session's board is current.
    // The id lookup is safe: the all view's card carries data-slug, and onBoard has made that board current.
    return onBoard(slugOf(sl), () => {
      const id = sl.dataset.sref.split('/').pop();
      const s = (state.sessions || []).find(x => x.id === id);
      if (s) openLinkDialog(s, sl.dataset.sessLink);
    });
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

registerView('columns', {
  render: () => renderColumns(),
  // Keyed by a bare task id: they belong to the board being left.
  reset: () => { for (const k of Object.keys(openReplies)) delete openReplies[k]; },
});
