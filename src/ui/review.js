// ── the review view ───────────────────────────────────────────────────

let focused = null;

/* How the review queue names a gate: its board and id, since gates of several boards share it.
   On a board of its own there is no board to name. */
const gateRef = g => g._slug ? `${g._slug}/${g.id}` : g.id;
/* The open gate or record a ref names; a plain id from an old link takes the first of that id. */
function gateByRef(ref) {
  const gates = state.gates || [];
  return gates.find(g => gateRef(g) === ref) || recordByRef(ref)
    || (ref && !String(ref).includes('/') ? gates.find(g => g.id === ref) : undefined);
}

/* A session waiting on a permission prompt or a question, listed beside the gates: it cannot be
   answered here, so its button opens that session's terminal. Named by board, row id and moment,
   since the wait is that pair. */
const waitRef = w => `${w._slug ? w._slug + '/' : ''}wait:${w.agentSessionId}:${w.since}`;
const itemRef = x => x._wait ? waitRef(x) : gateRef(x);
const reviewWaits = () => (state.waits || []).map(w => ({ ...w, _wait: true }));
const waitByRef = ref => reviewWaits().find(w => waitRef(w) === ref);
/* What the pill of a wait says: 許可待ち, 質問への回答待ち or 入力待ち. */
const waitLabel = w => permissionLabel({ agentSession: { request: w.request } });
const terminalIcon = '<span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">terminal</span>';

const renderDiff = d => esc(d).split('\n').map(l => {
  const cls = l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@') ? 'h'
            : l.startsWith('+') ? 'a' : l.startsWith('-') ? 'd' : '';
  return `<div class="${cls}">${l || ' '}</div>`;
}).join('');

/* ── the queue ── */

/* What was answered in this page, kept until it is reloaded: ref → { gate, decision, at }. It
   stays in the list, dimmed, under 処理済み, so a slip of the hand can be seen; it is not in the
   counts or on the 人 board, which go by what the boards still list. */
const reviewDone = new Map();
/* The unanswered refs in the order the list was last drawn: what 前へ, 次へ and the move after
   an answer go through. */
let reviewOrder = [];
/* 判断 or ターミナル. It starts on 判断 for every item. */
let reviewPane = 'judge';
let reviewShown = null;     // the ref the right-hand side is drawn for
let judgeFor = null;
let judgeScroll = 0;         // where #rv-judge was scrolled to when the terminal came over it
let judgeDirty = false;      // #rv-judge was due a redraw while it was hidden        // the ref #rv-judge holds the markup of
const reviewSig = { head: '', tabs: '', judge: '', bar: '', ph: '' };
const rv = id => document.getElementById(id);
function setReviewPart(part, el, html) {
  if (reviewSig[part] === html) return false;
  reviewSig[part] = html;
  el.innerHTML = html;
  return true;
}

/* The item on screen: what was answered here keeps the copy it had, the rest is read from the
   state, a record included. */
const reviewCurrent = () => reviewDone.get(focused)?.gate || gateByRef(focused) || waitByRef(focused);

/* The waiting gates and waiting sessions by board, in the sidebar's order, the longest-waiting
   first. A gate answered here is out even if a poll that was already on its way still lists it. */
function reviewGroups() {
  const open = [...(state.gates || []).filter(g => !reviewDone.has(gateRef(g))), ...reviewWaits()];
  const since = x => x._wait ? x.since : stampSecs(x.openedAt) ?? 0;
  const slugs = multiBoard ? orderedBoards(readBoards()).map(b => b.slug) : [''];
  for (const g of open) if (!slugs.includes(g._slug || '')) slugs.push(g._slug || '');
  return slugs.map(slug => {
    const items = open.filter(g => (g._slug || '') === slug)
      .sort((a, b) => since(a) - since(b) || itemRef(a).localeCompare(itemRef(b)));
    const b = boards.find(x => x.slug === slug);
    return { slug, items, title: b ? boardName(b) : repoName() || '', repo: b?.hub ? repoNameOf(b) : '' };
  }).filter(grp => grp.items.length);
}

/* A waiting session's row: a pill of its own kind, not a gate's, and when it began to wait. */
function reviewWaitRowHtml(w, cur) {
  const ref = waitRef(w);
  return `<button type="button" class="review-inbox-item item" data-rv-item="${esc(ref)}" aria-current="${ref === cur}">
    <div class="rv-row-top">
      <span class="m3-pill pill-neutral">${terminalIcon}${esc(waitLabel(w))}</span>
      <span class="w rv-row-when">${sinceLabel(w.since)}</span>
    </div>
    <div class="t">${esc(w.name)}</div>
    <div class="rv-row-wt">${esc(w.kind)}</div>
  </button>`;
}

function reviewRowHtml(g, cur, done) {
  if (g._wait) return reviewWaitRowHtml(g, cur);
  const ref = gateRef(g);
  const [label] = kindOf(g.kind);
  return `<button type="button" class="review-inbox-item item${done ? ' done' : ''}" data-rv-item="${esc(ref)}" aria-current="${ref === cur}">
    <div class="rv-row-top">
      <span class="m3-pill ${g.kind === 'plan' ? 'pill-blue' : g.kind === 'diff' ? 'pill-purple' : 'pill-warn'}">${esc(label)}</span>
      ${parkOf(taskOfGate(g)) ? '<span class="m3-pill pill-neutral">置いている</span>' : ''}
      <span class="w rv-row-when">${done ? esc(DECISION[done.decision === 'close' ? 'closed' : done.decision] || done.decision) : ago(g.openedAt)}</span>
    </div>
    <div class="t">${esc(g.title)}</div>
    <div class="rv-row-wt">${esc(baseName(g.worktree))}</div>
  </button>`;
}

function renderReviewList(groups, cur) {
  const list = document.querySelector('#review .review-inbox-list');
  const done = [...reviewDone.entries()].sort((a, b) => a[1].at - b[1].at);
  let html = '';
  for (const grp of groups) {
    html += `<div class="rv-group-head"><span class="rv-group-name">${esc(grp.title)}</span>`
      + (grp.repo ? `<span class="rv-group-repo">${esc(grp.repo)}</span>` : '')
      + `<span class="rv-group-count">${grp.items.length}</span></div>`
      + grp.items.map(g => reviewRowHtml(g, cur, null)).join('');
  }
  if (done.length) {
    html += `<div class="rv-group-head done"><span class="rv-group-name">処理済み</span><span class="rv-group-count">${done.length}</span></div>`
      + done.map(([, d]) => reviewRowHtml(d.gate, cur, d)).join('');
  }
  if (!html) html = '<div class="empty-state">要対応はありません</div>';
  rv('rv-count').textContent = reviewOrder.length;
  // Only the rows are replaced, so the list keeps where it was scrolled to.
  if (list.dataset.sig !== html) {
    list.dataset.sig = html;
    list.innerHTML = html;
  }
  if (list.dataset.cur !== String(cur)) {
    list.dataset.cur = String(cur);
    list.querySelector('[aria-current="true"]')?.scrollIntoView({ block: 'nearest' });
  }
}

/* The ref after `ref` that is still waiting: the next in the list, else the first that is left
   so that none is skipped. */
function nextUnansweredAfter(ref) {
  const live = r => !reviewDone.has(r) && ((state.gates || []).some(g => gateRef(g) === r) || !!waitByRef(r));
  const at = reviewOrder.indexOf(ref);
  return reviewOrder.slice(at + 1).find(live) || reviewOrder.find(r => r !== ref && live(r)) || null;
}

/* Called when a gate was answered. The next item is shown by replacing the address: going back
   from here should leave the queue, not step through what was just answered. */
function reviewAnswered(g, decision) {
  // Only what is answered here: on a board's own page a gate has no board in its ref, and a bare
  // id could hide another board's gate of the same id from the queue.
  if (g.wait === false || view !== 'review') return;
  const ref = gateRef(g);
  reviewDone.set(ref, { gate: { ...g }, decision, at: Date.now() });
  if (focused !== ref || !prefs.reviewNext) return;
  focused = nextUnansweredAfter(ref);
  reviewPane = 'judge';
  setNav({ item: focused });
}

/* A choice of the person (a click, 前へ, 次へ): a step in the history. */
function reviewSelect(ref) {
  if (!ref || ref === focused) return;
  focused = ref;
  reviewPane = 'judge';
  go({ item: ref });
}
function reviewStep(dir) {
  const at = reviewOrder.indexOf(focused);
  reviewSelect(at < 0 ? (dir > 0 ? reviewOrder[0] : null) : reviewOrder[at + dir]);
}

/* ── the item on screen ── */

function reviewHeadHtml(g, ref) {
  const done = reviewDone.get(ref);
  const [label] = g._wait ? [waitLabel(g)] : kindOf(g.kind);
  const b = boards.find(x => x.slug === g._slug);
  const at = reviewOrder.indexOf(ref);
  const prevOk = at > 0;
  const nextOk = at < 0 ? reviewOrder.length > 0 : at < reviewOrder.length - 1;
  return `<div class="rv-head-main">
      ${b ? `<span class="tag rv-board-tag">${esc(boardName(b))}</span>` : ''}
      <span class="m3-pill ${g._wait ? 'pill-neutral' : 'pill-warn'}">${esc(label)}</span>
      ${done ? '<span class="m3-pill pill-neutral">処理済み</span>' : ''}
      <span class="rv-head-title">${esc(g._wait ? g.name : g.title)}</span>
    </div>
    <div class="rv-head-nav">
      <button type="button" class="btn-m3-tonal" data-rv-step="-1"${prevOk ? '' : ' disabled'}><span class="material-symbols-outlined" aria-hidden="true">arrow_back</span><span>前へ</span></button>
      <span class="rv-pos">${at >= 0 ? `${at + 1} / ${reviewOrder.length}` : `${reviewOrder.length} 件`}</span>
      <button type="button" class="btn-m3-tonal" data-rv-step="1"${nextOk ? '' : ' disabled'}><span>次へ</span><span class="material-symbols-outlined" aria-hidden="true">arrow_forward</span></button>
      <label class="rv-next"><input type="checkbox" data-rv-next> 処理したら次へ</label>
    </div>`;
}

/* The hub of the item's board: the board's own, else (a board served alone) the one it lists. */
const reviewHubOf = g => g._slug ? boards.find(b => b.slug === g._slug)?.hubId : (pageHub()?.id || repoHubId());

/* The session behind the item, which `reviewTermUsable` and the terminal both go by: the board's
   hub for what the hub waits on, else the worker in the item's worktree. */
function reviewSessionOf(g) {
  const mine = (state.sessions || []).filter(s => !g._slug || s._slug === g._slug);
  if (g.answeredByHub) return mine.find(s => s.kind === 'hub' && s.id === reviewHubOf(g));
  return g.worktree ? mine.find(s => s.kind === 'worker' && s.worktree === g.worktree) : undefined;
}

/* Whether the ターミナル tab can open for this item, without listing the sessions: the board
   serves terminals and what the item waits on is running. */
function reviewTermUsable(g) {
  if (!state.boardTerminal?.available) return false;
  if (g.answeredByHub) {
    return g._slug ? !!boards.find(b => b.slug === g._slug)?.hubPresent
      : !!(state.hubs || []).find(h => h.id === reviewHubOf(g))?.state?.present;
  }
  return !!g.worktree && !!(state.workers || []).find(w => w.worktree === g.worktree)?.present;
}

/* Whether the sessions of the item's board are in: a board served alone always has them. */
const reviewSessionsRead = g => !scopeAll() || state.reviewSessionsOf === g._slug;

function reviewTabsHtml(g) {
  // A waiting session is answered in its terminal, which is not the board's to mount here.
  if (g._wait) return '';
  const usable = reviewTermUsable(g);
  const hint = state.boardTerminal?.available ? 'セッションなし' : '端末はボードから開けません';
  const tab = (id, label, extra, off) =>
    `<button type="button" role="tab" id="rv-tab-${id}" class="tp-tab${reviewPane === id ? ' on' : ''}" data-rv-pane="${id}" aria-selected="${reviewPane === id}" aria-controls="${id === 'term' ? 'rv-term' : 'rv-judge'}"${off ? ` disabled title="${esc(off)}"` : ''}>${label}${extra}</button>`;
  return tab('judge', '判断', '', '')
    + tab('term', 'ターミナル', usable ? '' : `<span class="tp-tab-hint">${esc(hint)}</span>`, !usable && hint);
}

/* Issue and PR of the task, a row each; a gate with no task has neither. */
function reviewRefsHtml(g, task) {
  return task ? `<div class="rv-refs" data-rv-refs>${ghRowsHtml(task)}</div>` : '';
}

/* What it takes to answer: the buttons the gate's options name, a comment box, 話す. */
function reviewDockHtml(g) {
  const btn = (act, cls, icon, text, style = '') =>
    `<button type="button" class="${cls}" data-act="${act}"${style ? ` style="${style}"` : ''}><span class="material-symbols-outlined" style="font-size:16px;">${icon}</span><span>${text}</span></button>`;
  const BUTTON = {
    approve: () => btn('approve', 'btn-m3-primary approve', 'check', decisionLabel('approve', g.kind),
      'background:var(--md-sys-color-success);color:var(--md-sys-color-on-success)'),
    changes: () => btn('changes', 'btn-m3-tonal changes', 'replay', decisionLabel('changes', g.kind)),
    reject: () => btn('reject', 'btn-m3-text reject', 'cancel', decisionLabel('reject', g.kind), 'color:var(--md-sys-color-error)'),
    ack: () => btn('ack', 'btn-m3-primary approve', 'check', decisionLabel('ack', g.kind)),
    answer: () => btn('answer', 'btn-m3-primary approve', 'send', decisionLabel('answer', g.kind)),
    ask: () => btn('ask', 'btn-m3-tonal changes', 'help', decisionLabel('ask', g.kind)),
  };
  return `<div class="decision-dock panel" data-gate="${esc(gateRef(g))}">
    ${roundsHintHtml(g)}
    <textarea class="gate-comment" placeholder="修正指示や質問があれば入力してください（承認の場合は空欄でも可）..."></textarea>
    <div class="decide">
      ${(g.options || []).map(o => (BUTTON[o] || (() => ''))()).join('')}
      <button type="button" class="m3-icon-button talk" style="padding:8px 14px" data-rv-talk><span class="material-symbols-outlined" style="font-size:16px;">terminal</span><span>ターミナルで話す</span></button>
      ${g.worktree && g.kind !== 'verify' ? `<button type="button" class="m3-icon-button" style="padding:8px 14px" title="${ideTitle()}" data-ide="${esc(g.worktree)}"><span class="material-symbols-outlined" style="font-size:16px;">code</span><span>IDEで開く</span></button>` : ''}
      ${parkButtonHtml(g)}
      <button type="button" class="btn-m3-text close" style="margin-left:auto;color:var(--md-sys-color-outline)" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします"><span class="material-symbols-outlined" style="font-size:16px;">done_all</span><span>解決済みとして閉じる</span></button>
    </div>
  </div>`;
}

/* The 判断 tab: one column, from what waits to the buttons that answer it. */
/* A session waiting on a prompt or a question: what it waits on, and the way to its terminal. */
function reviewWaitJudgeHtml(w) {
  const asked = { agentSession: { request: w.request } };
  return `<div class="m3-card-attention-box rv-wait">
    <div class="rv-wait-head">
      <span class="material-symbols-outlined" style="font-size:18px;" aria-hidden="true">terminal</span>
      <span>【${esc(waitLabel(w))}】ターミナルで入力を待っています</span>
      <span class="rv-wait-when">${sinceLabel(w.since)}から待ち</span>
    </div>
    <div class="rv-wait-title">${esc(w.name)}</div>
    ${w.request ? `<div class="rv-wait-why">${esc(requestText(asked))}</div>` : ''}
  </div>
  <div><button type="button" class="btn-m3-primary" data-rv-open-wait>${terminalIcon}<span>ターミナルを開く</span></button></div>`;
}

function reviewJudgeHtml(g, task, done) {
  if (g._wait) return reviewWaitJudgeHtml(g);
  const record = g.wait === false;
  const [label] = kindOf(g.kind);
  let h = reviewRefsHtml(g, task);

  const why = g.problem || g.why || '';
  h += `<div class="m3-card-attention-box rv-wait">
    <div class="rv-wait-head">
      <span class="material-symbols-outlined" style="font-size:18px;">${record ? 'history' : 'pending_actions'}</span>
      <span>【${esc(label)}】${record ? '記録 — worker は止まらずに進んだ' : 'あなたの判定待ち'}</span>
      <span class="rv-wait-when">${ago(g.openedAt)}${record ? 'に記録' : 'から待ち'}</span>
    </div>
    <div class="rv-wait-title">${esc(g.title)}</div>
    ${why ? `<div class="rv-wait-why">${esc(why)}</div>` : ''}
    ${stopWhy(g).length ? `<div><strong>止めた理由:</strong><ul>${stopWhy(g).map(w =>
      `<li${stopBad(g) ? ' style="color:var(--md-sys-color-error)"' : ''}>${esc(w)}</li>`).join('')}</ul></div>` : ''}
    ${g.focus ? `<div class="rv-wait-focus"><strong>確認してほしい点:</strong> ${md(g.focus)}</div>` : ''}
  </div>`;
  if (!record) h += parkBannerHtml(task);

  if (g.decided) {
    h += `<div class="panel"><details class="decided"><summary style="font-weight:700;cursor:pointer;">決定事項</summary><div class="body" style="margin-top:8px;">${md(g.decided)}</div></details></div>`;
  }

  const pickable = !record && !done;
  if (g.kind === 'diff') {
    h += reviewPanels(g);
    if (g.diff) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">difference</span><span>コード差分 (Diff)</span></h3>
        <div class="diff">${renderDiff(g.diff)}</div>
      </div>`;
    } else if (diffPending(g)) {
      h += `<div class="empty-state">差分を読み込み中…</div>`;
    } else if (!g.findings?.length && !g.reviewRounds?.length) {
      h += `<div class="empty-state">この Gate に記録されたコード差分はありません。</div>`;
    }
  } else if (g.kind === 'verify') {
    if (pickable) {
      h += `<div class="work">
        <button class="big" title="${ideTitle()}" data-ide="${esc(g.worktree)}">
          <span class="material-symbols-outlined" style="font-size:16px;vertical-align:text-bottom;margin-right:4px;">code</span>
          <span>IDE で開く</span>
        </button>
        <span class="mono2">${esc(g.worktree)}</span>
        <span style="color:var(--md-sys-color-on-surface-variant);font-size:12px;">— 確認後、下のパネルで判定してください</span></div>`;
    }
    h += checkPanels(g);
    if (g.run) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">play_arrow</span><span>動かし方</span></h3>
        <div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div>
      </div>`;
    }
  } else {
    if (task?.body) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">assignment</span><span>依頼内容 / 要件プロンプト</span></h3>
        <div class="body">${md(task.body)}</div>
      </div>`;
    }
    if (g.body && g.body !== task?.body) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">report</span><span>Gate 報告</span></h3>
        <div class="body">${md(g.body)}</div>
      </div>`;
    }
    if (g.facts?.length) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">info</span><span>事実</span></h3>
        <ul>${g.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul>
      </div>`;
    }
    if (g.unsure) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">help</span><span>迷っていること</span></h3>
        <div class="body">${md(g.unsure)}</div>
      </div>`;
    }
    h += choicesHtml(g, pickable);
  }

  if (task) {
    h += `<div><button type="button" class="btn-m3-text" data-rv-history><span>経過をすべて見る</span><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">arrow_forward</span></button></div>`;
  }
  if (done) {
    h += `<div class="decision-dock panel rv-done-note"><span class="material-symbols-outlined" style="font-size:18px;" aria-hidden="true">task_alt</span><span>処理済み: ${esc(DECISION[done.decision === 'close' ? 'closed' : done.decision] || done.decision)}</span></div>`;
  } else {
    // A record is sent back from the same form the task panel has.
    h += record ? decideHtml(g) : reviewDockHtml(g);
  }
  return h;
}

/* #rv-judge is the only part drawn with its comment box inside: a redraw that comes while it is
   being typed in is held (redrawReview), as it cuts an IME composition short. */
function renderReviewJudge(g, ref) {
  const el = rv('rv-judge');
  const done = reviewDone.get(ref);
  const task = taskOfGate(g) || (state.tasks || []).find(t => t.worktree && t.worktree === g.worktree && (!g._slug || t._slug === g._slug));
  const html = reviewJudgeHtml(g, task, done);
  const sameItem = judgeFor === ref;
  // Not drawn under the terminal: a hidden box has no scroll to keep. It is drawn when shown.
  if (el.hidden && reviewPane === 'term' && sameItem) { judgeDirty = true; return; }
  if (!sameItem) judgeScroll = 0;
  // Another item starts at the top, with a box of its own.
  const scroll = sameItem ? el.scrollTop : 0;
  const comment = el.querySelector('.gate-comment');
  const value = sameItem && comment ? comment.value : '';
  const wasFocused = !!comment && document.activeElement === comment;
  const wasDecidedOpen = sameItem && el.querySelector('details.decided')?.open;
  if (sameItem && reviewSig.judge === html) return;
  reviewSig.judge = html;
  judgeFor = ref;
  el.innerHTML = html;
  bindDecide(el);
  if (wasDecidedOpen) el.querySelector('details.decided')?.setAttribute('open', '');
  restoreComment(el, value, wasFocused, scroll);
}

/* The ターミナル tab. The host is never drawn again: only the bar and the note over it are. */
function renderReviewTerm() {
  if (view !== 'review') return;
  const g = reviewCurrent();
  const ref = g ? itemRef(g) : null;
  // Shown before the terminal is mounted: a hidden host has no size to fit to.
  const wantTerm = !!g && reviewPane === 'term';
  const reveal = wantTerm && rv('rv-term').hidden;
  const judge = rv('rv-judge');
  if (wantTerm && !judge.hidden) judgeScroll = judge.scrollTop;
  const unhide = !wantTerm && judge.hidden;
  judge.hidden = wantTerm;
  rv('rv-term').hidden = !wantTerm;
  if (unhide) {
    judge.scrollTop = judgeScroll;
    if (judgeDirty) { judgeDirty = false; renderReview(); return; }
  }
  const s = g ? reviewSessionOf(g) : null;
  syncTermSlot(reviewTerm, ref, s, wantTerm ? 'term' : 'judge');
  if (!wantTerm) return;
  setReviewPart('bar', rv('rv-term-bar'), termBarHtml(s, reviewTerm, false));
  const ph = rv('rv-term-ph');
  ph.hidden = !!reviewTerm.term;
  // Until the sessions are in, a usable tab is only waiting for them; once read, none is none.
  setReviewPart('ph', ph, reviewTerm.term ? '' : !s && reviewTermUsable(g) ? `<div class="tp-muted">${reviewSessionsRead(g) ? 'セッションはありません' : '接続しています…'}</div>` : termPlaceholderHtml(s));
  if (reveal && reviewTerm.term) {
    // It was sized while hidden, which it skips; asked again now that it has a size.
    reviewTerm.term.fit();
    reviewTerm.term.focus();
  }
}

function setReviewPane(pane) {
  if (reviewPane === pane) return;
  reviewPane = pane;
  const g = reviewCurrent();
  if (g) setReviewPart('tabs', rv('rv-tabs'), reviewTabsHtml(g));
  renderReviewTerm();
  // The sessions of this item's board are asked for while the terminal is open.
  if (pane === 'term') refresh(true);
}

/* `holdJudge`: leave #rv-judge as it is, for a comment being typed in it. */
function renderReview({ holdJudge = false } = {}) {
  reviewHeld = holdJudge;
  const groups = reviewGroups();
  reviewOrder = groups.flatMap(grp => grp.items.map(itemRef));
  // Nothing is known before the first round: say so rather than "nothing left", and leave the
  // address alone, so the item it names is still there to be found.
  if (scopeAll() ? !allRound : state.now == null) {
    renderReviewList(groups, null);
    setReviewPart('head', rv('rv-head'), '');
    setReviewPart('tabs', rv('rv-tabs'), '');
    judgeFor = null;
    reviewSig.judge = '';
    rv('rv-judge').innerHTML = '<div class="empty-state rv-empty">読み込んでいます…</div>';
    // Back from a visit with the terminal open: nothing of it stays on screen.
    rv('rv-judge').hidden = false;
    rv('rv-term').hidden = true;
    reviewSig.bar = '';
    reviewSig.ph = '';
    return;
  }
  let g = reviewCurrent();
  if (!g) {
    focused = reviewOrder[0] || null;
    g = reviewCurrent();
  }
  const ref = g ? itemRef(g) : null;
  renderReviewList(groups, ref);
  if (!g) {
    reviewShown = null;
    judgeFor = null;
    reviewPane = 'judge';
    setReviewPart('head', rv('rv-head'), '');
    setReviewPart('tabs', rv('rv-tabs'), '');
    reviewSig.judge = '';
    rv('rv-judge').innerHTML = `<div class="empty-state rv-empty">
      <span class="material-symbols-outlined" style="font-size:48px;opacity:.5;display:block;margin-bottom:12px;" aria-hidden="true">done_all</span>
      <div style="font-size:15px;font-weight:700;color:var(--md-sys-color-on-surface);">${reviewDone.size ? 'すべて処理しました' : '対応待ちはありません'}</div>
      <div style="font-size:13px;margin-top:6px;">${reviewDone.size ? `処理済み ${reviewDone.size} 件` : ''}</div></div>`;
    renderReviewTerm();
    if (view === 'review' && nav.item) setNav({ item: null });
    return;
  }
  focused = ref;
  if (reviewShown !== ref) { reviewShown = ref; reviewPane = 'judge'; }
  const record = g.wait === false;
  if (record && view === 'review') markSeen(ref);
  // A permalink, so "反映しといたから見てね" can point at this one card.
  if (view === 'review' && nav.item !== ref) setNav({ item: ref });
  // A polled record has no diff of its own; its task's history has it.
  const shown = record ? withDiff(g, taskOfGate(g)) : g;

  // A control of the head that has the keyboard focus keeps it across the redraw.
  const head = rv('rv-head');
  const at = document.activeElement;
  const focusKey = head.contains(at) ? (at.dataset.rvStep ? `[data-rv-step="${at.dataset.rvStep}"]` : at.matches('[data-rv-next]') ? '[data-rv-next]' : null) : null;
  if (setReviewPart('head', head, reviewHeadHtml(shown, ref)) && focusKey) head.querySelector(focusKey)?.focus();
  // The checkbox is set apart from the markup, so a toggle does not redraw the head.
  head.querySelector('[data-rv-next]').checked = !!prefs.reviewNext;
  setReviewPart('tabs', rv('rv-tabs'), reviewTabsHtml(shown));
  // The item shown in the box is the one the comment is for: if it changed, the hold ends.
  if (!(holdJudge && judgeFor === ref)) {
    reviewHeld = false;
    renderReviewJudge(shown, ref);
  }
  renderReviewTerm();
}

/* A redraw that came while a comment was being typed in the review view: the list and the head
   can be drawn, #rv-judge is held until the box is left, as the task panel does: replacing the
   markup under the box cuts an IME composition short even when the text is put back. A person's
   own action still redraws at once. */
let reviewHeld = false;
function redrawReview() {
  if (view !== 'review') return;
  renderReview({ holdJudge: !!document.activeElement?.matches('#rv-judge .gate-comment') });
}
document.getElementById('review').addEventListener('focusout', e => {
  if (!reviewHeld || !e.target.matches('.gate-comment')) return;
  // After the focus has moved: a click on a button redraws through its own handler, and this
  // one should not draw over it with the old state.
  setTimeout(() => {
    if (!reviewHeld || document.activeElement?.matches('#rv-judge .gate-comment')) return;
    renderReview();
  });
});

document.getElementById('review').addEventListener('click', e => {
  let b;
  if ((b = e.target.closest('[data-rv-item]'))) return reviewSelect(b.dataset.rvItem);
  if ((b = e.target.closest('[data-rv-step]')) && !b.disabled) return reviewStep(+b.dataset.rvStep);
  if ((b = e.target.closest('[data-rv-pane]')) && !b.disabled) return setReviewPane(b.dataset.rvPane);
  if (e.target.closest('[data-tp-reconnect]')) { reviewTerm.reconnect = true; return renderReviewTerm(); }
  if (e.target.closest('[data-rv-talk]')) {
    const g = reviewCurrent();
    // Where the terminal can open it is this tab; elsewhere, the outside tab is brought forward.
    if (g && reviewTermUsable(g)) setReviewPane('term'); else talk(focused);
    return;
  }
  if (parkClick(e)) return;
  if (e.target.closest('[data-rv-open-wait]')) {
    const w = reviewCurrent();
    if (w?._wait) openWait(w);
    return;
  }
  if (e.target.closest('[data-rv-history]')) {
    const g = reviewCurrent();
    const task = g && taskOfGate(g);
    if (task) onBoard(g._slug, () => openTask(task.id, TAB_OF_KIND[g.kind] || 'history', g.id));
  }
});
document.getElementById('review').addEventListener('change', e => {
  if (!e.target.matches('[data-rv-next]')) return;
  prefs.reviewNext = e.target.checked;
  savePrefs();
});

/* The task a gate belongs to; with several boards, the one on the gate's own. */
function taskOfGate(g) {
  return (state.tasks || []).find(t => t.id === g.task && (!g._slug || t._slug === g._slug));
}

registerView('review', { render: () => redrawReview(), reset: () => disposeTermSlot(reviewTerm) });
