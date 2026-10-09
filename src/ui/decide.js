/* Judging a gate, wherever it is judged (the task panel, a hub's or a session's, a gate's own): how a gate is named, what was
   answered in this page, and the screens and buttons that answer one. */

/* How a gate is named: its board and id, since gates of several boards share it.
   On a board of its own there is no board to name. */
const gateRef = g => g._slug ? `${g._slug}/${g.id}` : g.id;
/* The open gate or record a ref names; a plain id from an old link takes the first of that id. */
function gateByRef(ref) {
  const gates = state.gates || [];
  return gates.find(g => gateRef(g) === ref) || recordByRef(ref)
    || (ref && !String(ref).includes('/') ? gates.find(g => g.id === ref) : undefined);
}

const renderDiff = d => esc(d).split('\n').map(l => {
  const cls = l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@') ? 'h'
            : l.startsWith('+') ? 'a' : l.startsWith('-') ? 'd' : '';
  return `<div class="${cls}">${l || ' '}</div>`;
}).join('');

/* What was answered in this page, kept until it is reloaded: key → { at }. Not in
   the counts or on the 人 board, which go by what the boards still list, so an answer that a poll
   already on its way does not know yet does not bring the gate back. */
const answeredGates = new Map();
/* The key a gate is kept under: its board and id, as `gateRef` names it. A state that is one
   board's own tags no board on its gates, so the board shown is theirs; a board served alone has
   none to name. */
const gateKey = g => gateRef(g._slug || !multiBoard || !nav.board || nav.board === 'all' ? g : { ...g, _slug: nav.board });

/* Called when a gate was answered or closed, in whichever view. `key` is `gateKey(g)` as it was when the answer was sent: the
   person may have moved to another board while it was on its way. */
function gateAnswered(g, key) {
  if (g.wait === false) return;
  answeredGates.set(key, { at: Date.now() });
  // 「処理したら次へ」 in 「いまの仕事」.
  workAdvanceAfter(key);
}

/* The task a gate belongs to; with several boards, the one on the gate's own. */
function taskOfGate(g) {
  return (state.tasks || []).find(t => t.id === g.task && (!g._slug || t._slug === g._slug));
}

/* Each button names its decision in `data-act`; `bindDecide` finds the gate it belongs to from
   the `data-gate` around it, so the same panel works wherever a gate is judged. */
const BUTTONS = {
  approve: g => `<button class="btn-m3-primary approve" data-act="approve" style="background:var(--md-sys-color-success);color:var(--md-sys-color-on-success)"><span class="material-symbols-outlined" style="font-size:16px;">check</span><span>${decisionLabel('approve', g.kind)}</span></button>`,
  changes: g => `<button class="btn-m3-tonal changes" data-act="changes"><span class="material-symbols-outlined" style="font-size:16px;">replay</span><span>${decisionLabel('changes', g.kind)}</span></button>`,
  reject:  g => `<button class="btn-m3-text reject" data-act="reject" style="color:var(--md-sys-color-error);"><span class="material-symbols-outlined" style="font-size:16px;">cancel</span><span>${decisionLabel('reject', g.kind)}</span></button>`,
  ack:     g => `<button class="btn-m3-primary approve" data-act="ack"><span class="material-symbols-outlined" style="font-size:16px;">check</span><span>${decisionLabel('ack', g.kind)}</span></button>`,
  ask:     g => `<button class="btn-m3-tonal changes" data-act="ask"><span class="material-symbols-outlined" style="font-size:16px;">help</span><span>${decisionLabel('ask', g.kind)}</span></button>`,
  answer:  g => `<button class="btn-m3-primary approve" data-act="answer"><span class="material-symbols-outlined" style="font-size:16px;">send</span><span>${decisionLabel('answer', g.kind)}</span></button>`,
};

/* The nudge toward the terminal once a gate has gone back and forth. */
const roundsHintHtml = g => (g.rounds || 0) >= 2 ? `<div class="hint-bar" style="display:flex;align-items:center;gap:6px;margin-bottom:12px;">
  <span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">warning</span>
  <span>ここまで ${g.rounds} 往復しています。<b>ターミナルで直接やり取りしたほうが円滑</b>です（1往復ごとに outbox と wake を経由します）</span>
</div>` : '';

/* The attributes that say which task a park button is for (`parkClick`, actions.js). */
const parkAttrs = (t, off) => `data-park-task="${esc(t.id)}" data-park-board="${esc(t._slug || '')}"${off ? ' data-park-off' : ''}`;
/* The button next to a gate's answers that parks the task the gate names, or takes the park back: not drawn for a gate whose task
   this page does not have, nor for a finished task. The gate stays open and is answered as usual. */
function parkButtonHtml(g) {
  const t = taskOfGate(g);
  if (!t || ['done', 'cancelled'].includes(t.status)) return '';
  return parkOf(t)
    ? `<button type="button" class="m3-icon-button" style="padding:8px 14px" ${parkAttrs(t, true)}><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">alarm_off</span><span>置くのをやめる</span></button>`
    : `<button type="button" class="m3-icon-button" style="padding:8px 14px" ${parkAttrs(t, false)} title="誰かの返事やタイミングを待つので、「いまの仕事」の新着から外して置く"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">schedule</span><span>置く</span></button>`;
}
/* 「置いている — PdM の確認待ち（…） · 4分前から」 with the way to take it back, above a gate that stays open. */
function parkBannerHtml(task) {
  const p = parkOf(task);
  if (!p) return '';
  const since = stampSecs(p.since) != null ? ` · ${esc(ago(p.since))}から` : '';
  return `<div class="park-banner" role="status"><span class="material-symbols-outlined" aria-hidden="true">schedule</span><span class="park-banner-text">置いている — ${esc(parkText(p))}${since}</span><button type="button" class="btn-m3-text" ${parkAttrs(task, true)}>置くのをやめる</button></div>`;
}

/* The part of a gate a person acts on: the send-back form for a record, the decision for a
   gate that waits. */
function decideHtml(g) {
  if (g.wait === false) {
    return `<div class="decision-dock panel" data-gate="${esc(gateRef(g))}">
      <h3 style="font-size:14px;font-weight:800;display:flex;align-items:center;gap:6px;color:var(--md-sys-color-on-surface);">
        <span class="material-symbols-outlined" style="font-size:18px;color:var(--md-sys-color-primary);">replay</span>
        <span>${decisionLabel('changes', g.kind)}</span>
      </h3>` +
      ((g.answers || []).length ? `<ul style="margin:0 0 12px;padding-left:18px">${g.answers.map(a =>
        `<li><span style="color:var(--md-sys-color-outline)">${ago(a.answeredAt)}に差し戻し</span>${a.comment ? ` — ${esc(a.comment)}` : ''}</li>`).join('')}</ul>` : '') +
      `<div class="field" style="margin-bottom:12px">
        <label style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);margin-bottom:4px;display:block;">コメント（何を直してほしいか）</label>
        <textarea class="gate-comment" placeholder="理由を入力してください..."></textarea>
      </div>
      <div class="decide">
        <button class="btn-m3-tonal changes" data-act="changes">
          <span class="material-symbols-outlined" style="font-size:16px;">replay</span>
          <span>${decisionLabel('changes', g.kind)}</span>
        </button>
        ${g.worktree ? `
          <button class="m3-icon-button" style="padding:8px 14px" data-focus="${esc(g.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">terminal</span>
            <span>ターミナルで話す</span>
          </button>
          <button class="m3-icon-button" style="padding:8px 14px" title="${ideTitle()}" data-ide="${esc(g.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">code</span>
            <span>IDEで開く</span>
          </button>
        ` : ''}
      </div>
      <p style="color:var(--md-sys-color-outline);font-size:11.5px;margin-top:10px">
        worker はこの記録を残して先に進んでいます。差し戻すと <code>adj gate answer</code> → 対象 worktree の outbox に追記 → <code>workerWake</code> で worker に通知します。記録はそのまま残り、差し戻しが追記されます。</p></div>`;
  }
  return `<div class="decision-dock panel" data-gate="${esc(gateRef(g))}">
    <h3 style="font-size:14px;font-weight:800;display:flex;align-items:center;gap:6px;color:var(--md-sys-color-on-surface);">
      <span class="material-symbols-outlined" style="font-size:18px;color:var(--md-sys-color-primary);">gavel</span>
      <span>${g.kind === 'verify' ? '確かめたら（判定）' : 'あなたの判定・指示'}</span>
    </h3>` +
    roundsHintHtml(g) +
    `<div class="field" style="margin-bottom:12px">
      <label style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);margin-bottom:4px;display:block;">コメント（修正指示・追加で聞きたいことはここに）</label>
      <textarea class="gate-comment" placeholder="修正指示や質問があれば入力してください（承認の場合は空欄でも可）..."></textarea>
    </div>
    <div class="decide">
      ${(g.options || []).map(o => (BUTTONS[o] || (() => ''))(g)).join('')}
      <button class="m3-icon-button talk" style="padding:8px 14px" data-act="talk">
        <span class="material-symbols-outlined" style="font-size:16px;">terminal</span>
        <span>ターミナルで話す</span>
      </button>
      ${g.worktree ? `
        <button class="m3-icon-button" style="padding:8px 14px" title="${ideTitle()}" data-ide="${esc(g.worktree)}">
          <span class="material-symbols-outlined" style="font-size:16px;">code</span>
          <span>IDEで開く</span>
        </button>
      ` : ''}
      ${parkButtonHtml(g)}
      <button class="btn-m3-text close" style="margin-left:auto;color:var(--md-sys-color-outline)" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします">
        <span class="material-symbols-outlined" style="font-size:16px;">done_all</span>
        <span>解決済みとして閉じる</span>
      </button>
    </div>
    <div style="font-size:11.5px;color:var(--md-sys-color-outline);display:flex;align-items:flex-start;gap:6px;margin-top:8px;">
      <span class="material-symbols-outlined" style="font-size:14px;margin-top:2px;">info</span>
      <span>
        ${g.answeredByHub
          ? '判定は <code>adj gate answer</code> → hub の受信箱に送信 → <code>hubWake</code> で hub に通知します。'
          : '判定は <code>adj gate answer</code> → 対象 worktree の outbox に追記 → <code>workerWake</code> で worker に通知します。'}<br>
        <b>「ターミナルで話す」</b>は gate を開いたまま${g.answeredByHub ? ' hub' : ' worker'} タブを前面表示します。直接確認した後は<b>「解決済みとして閉じる」</b>を押してください（${g.answeredByHub ? 'hub への配信' : 'worker への outbox 配信'}なしでアーカイブします）。
      </span>
    </div>
  </div>`;
}

/* ── Judging a gate with no task card ────────────────────────────────────────────────────────
   A gate of a task is judged in that task's panel. One with no task on the board (the hub's own
   dispatch, issue or question gate, or a gate whose task is gone) is judged with these, in the
   panel of the gate, the hub or the session that waits on it. */

/* The gate's task, found by the id it names, else by the worktree it was opened in. */
function taskForGate(g) {
  return taskOfGate(g) || (state.tasks || []).find(t => t.worktree && t.worktree === g.worktree && (!g._slug || t._slug === g._slug));
}

/* What it takes to answer: the buttons the gate's options name, a comment box, 話す. */
function gateDockHtml(g) {
  const btn = (act, cls, icon, text, style = '') =>
    `<button type="button" class="${cls}" data-act="${act}"${style ? ` style="${style}"` : ''}><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">${icon}</span><span>${text}</span></button>`;
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
    <textarea class="gate-comment" aria-label="コメント" placeholder="修正指示や質問があれば入力してください（承認の場合は空欄でも可）..."></textarea>
    <div class="decide">
      ${(g.options || []).map(o => (BUTTON[o] || (() => ''))()).join('')}
      <button type="button" class="m3-icon-button talk" style="padding:8px 14px" data-act="talk"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">terminal</span><span>ターミナルで話す</span></button>
      ${g.worktree && g.kind !== 'verify' ? `<button type="button" class="m3-icon-button" style="padding:8px 14px" title="${ideTitle()}" data-ide="${esc(g.worktree)}"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">code</span><span>IDEで開く</span></button>` : ''}
      ${parkButtonHtml(g)}
      <button type="button" class="btn-m3-text close" style="margin-left:auto;color:var(--md-sys-color-outline)" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">done_all</span><span>解決済みとして閉じる</span></button>
    </div>
  </div>`;
}

/* The body of judging a gate, from what waits to the buttons that answer it: one column. `task` is
   the gate's task when there is one (its Issue and PR, its request). */
function gateJudgeHtml(g, task) {
  const record = g.wait === false;
  const [label] = kindOf(g.kind);
  // Issue and PR of the task, a row each; a gate with no task has neither.
  let h = task ? `<div class="rv-refs" data-rv-refs>${ghRowsHtml(task)}</div>` : '';

  const why = g.problem || g.why || '';
  h += `<div class="m3-card-attention-box rv-wait"${expandAttrs('wait', g)}>
    ${expandBtnHtml(`${label}・確認待ち`)}
    <div class="rv-wait-head">
      <span class="material-symbols-outlined" style="font-size:18px;" aria-hidden="true">${record ? 'history' : 'pending_actions'}</span>
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
    h += `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<details class="decided"><summary style="font-weight:700;cursor:pointer;">決定事項</summary><div class="body" style="margin-top:8px;">${md(g.decided)}</div></details></div>`;
  }

  const pickable = !record;
  if (g.kind === 'diff') {
    h += reviewPanels(g);
    if (g.diff) {
      h += `<div class="panel"${expandAttrs('diff', g)}>
        ${expandBtnHtml('コード差分')}
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
        <button type="button" class="big" title="${ideTitle()}" data-ide="${esc(g.worktree)}">
          <span class="material-symbols-outlined" style="font-size:16px;vertical-align:text-bottom;margin-right:4px;" aria-hidden="true">code</span>
          <span>IDE で開く</span>
        </button>
        <span class="mono2">${esc(g.worktree)}</span>
        <span style="color:var(--md-sys-color-on-surface-variant);font-size:12px;">— 確認後、下のパネルで判定してください</span></div>`;
    }
    h += checkPanels(g);
    if (g.run) {
      h += `<div class="panel"${expandAttrs('run', g)}>
        ${expandBtnHtml('動かし方')}
        <h3><span class="material-symbols-outlined" style="font-size:18px;">play_arrow</span><span>動かし方</span></h3>
        <div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div>
      </div>`;
    }
  } else {
    if (task?.body) {
      h += `<div class="panel"${expandAttrs('request', g)}>
        ${expandBtnHtml('依頼内容 / 要件プロンプト')}
        <h3><span class="material-symbols-outlined" style="font-size:18px;">assignment</span><span>依頼内容 / 要件プロンプト</span></h3>
        <div class="body">${md(task.body)}</div>
      </div>`;
    }
    if (g.body && g.body !== task?.body) {
      h += `<div class="panel"${expandAttrs('report', g)}>
        ${expandBtnHtml('Gate 報告')}
        <h3><span class="material-symbols-outlined" style="font-size:18px;">report</span><span>Gate 報告</span></h3>
        <div class="body">${md(g.body)}</div>
      </div>`;
    }
    if (g.facts?.length) {
      h += `<div class="panel"${expandAttrs('facts', g)}>
        ${expandBtnHtml('事実')}
        <h3><span class="material-symbols-outlined" style="font-size:18px;">info</span><span>事実</span></h3>
        <ul>${g.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul>
      </div>`;
    }
    if (g.unsure) {
      h += `<div class="panel"${expandAttrs('unsure', g)}>
        ${expandBtnHtml('迷っていること')}
        <h3><span class="material-symbols-outlined" style="font-size:18px;">help</span><span>迷っていること</span></h3>
        <div class="body">${md(g.unsure)}</div>
      </div>`;
    }
    h += choicesHtml(g, pickable);
  }

  // A record is sent back from the same form the task panel has.
  h += record ? decideHtml(g) : gateDockHtml(g);
  return h;
}

const deciding = new Set(); // gates with an answer in flight: a second click must not send it again
/* What a button of `decideHtml` or a gate's choices does, for the gate it sits in. True when the
   answer went, so a dialog that showed it knows to close. The task panel
   calls this from its own delegated click handler instead of `bindDecide`. */
async function decideAct(b) {
  const id = b.closest('[data-gate]').dataset.gate;
  const act = b.matches('.pick[data-choice]') ? 'choice' : b.dataset.act;
  if (act === 'talk') { talk(id); return false; }
  if (deciding.has(id)) return false;
  deciding.add(id);
  try {
    if (act === 'choice') return await answer('choice', b.dataset.choice, id);
    if (act === 'close') return await closeGate(id);
    return await answer(act, undefined, id);
  } finally { deciding.delete(id); }
}

/* The buttons `decideHtml` and a gate's choices draw, wired to the gate they sit in. */
function bindDecide(root) {
  root.querySelectorAll('[data-gate] [data-act], [data-gate] .pick[data-choice]').forEach(b =>
    b.addEventListener('click', () => decideAct(b)));
  root.querySelectorAll('button[data-ide]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('ide', b.dataset.ide)));
  root.querySelectorAll('button[data-focus]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('focus', b.dataset.focus)));
}

/* A gate's designs side by side. Only an open gate can still be answered with one; once
   answered, the one picked is marked instead. */
function choicesHtml(g, pickable) {
  if (!g.choices?.length) return '';
  return `<div class="panel"${expandAttrs('choices', g)}${pickable ? ` data-gate="${esc(gateRef(g))}"` : ''}>
    ${expandBtnHtml('AI からの提案・選択肢')}
    <h3 style="font-size:13px;font-weight:800;margin-bottom:12px;display:flex;align-items:center;gap:6px;color:var(--md-sys-color-primary);">
      <span class="material-symbols-outlined" style="font-size:18px;">lightbulb</span>
      <span>AI からの提案・選択肢</span>
    </h3>
    <div class="choices-container choices">` +
    g.choices.map(c => `
      <div class="m3-choice-card choice${c.recommended ? ' recommended rec' : ''}">
        <div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap;">
          ${c.recommended ? '<span class="m3-pill pill-blue rec-tag" style="align-self:flex-start;display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:14px;">auto_awesome</span><span>AI の推し案</span></span>' : ''}
          ${g.choice === c.id ? '<span class="m3-pill pill-good" style="align-self:flex-start;display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:14px;">check_circle</span><span>選ばれた案</span></span>' : ''}
        </div>
        <h4 style="margin:0;font-size:14px;font-weight:700;">${esc(c.label)}</h4>
        ${c.why ? `<p class="why" style="font-size:12px;color:var(--md-sys-color-on-surface-variant);margin:0;">${esc(c.why)}</p>` : ''}
        <ul style="padding-left:18px;font-size:12px;line-height:1.6;margin:0;">${(c.points || []).map(p => `<li>${esc(p)}</li>`).join('')}</ul>
        ${pickable ? `<button class="m3-btn-pick pick" data-choice="${esc(c.id)}"><span class="material-symbols-outlined" style="font-size:16px;">check</span><span>この案で進める</span></button>` : ''}
      </div>`).join('') + `</div></div>`;
}

/* What a redraw would otherwise lose: the comment being typed, and where the pane was. */
function restoreComment(pane, value, wasFocused, scroll) {
  const el = pane.querySelector('.gate-comment');
  if (el) {
    if (value) el.value = value;
    if (wasFocused) el.focus();
  }
  pane.scrollTop = scroll;
}

/* The structured fields a diff or verify gate carries beside its prose, open or recorded. */
const OUTCOME = { open:['未対応', 0], fixed:['修正済', 1], declined:['誤検知', 2] };
const structuredPanels = g => reviewPanels(g) + checkPanels(g);

function reviewPanels(g) {
  let h = '';
  const rounds = g.reviewRounds || [];
  if (rounds.length) {
    h += `<div class="panel"${expandAttrs('rounds', g)}>
      ${expandBtnHtml('セルフレビュー')}
      <h3><span class="material-symbols-outlined" style="font-size:18px;">rate_review</span><span>セルフレビュー ${rounds.length}ラウンド</span></h3>
      <div class="body"><table>
      <tr><th>R</th><th>エンジン</th><th>must</th><th>want</th><th>scope</th><th>誤検知</th></tr>` +
      rounds.map((r, i) => `<tr><td>R${i + 1}</td><td>${esc(r.engine)}</td><td>${r.must || 0}</td>` +
        `<td>${r.want || 0}</td><td>${r.scope || 0}</td><td>${r.falsePositives || 0}</td></tr>`).join('') +
      `</table></div></div>`;
  }
  // Open first, since those are what is left; then fixed; then the false positives, each with
  // why it was declined.
  const findings = g.findings || [];
  if (findings.length) {
    const groups = [...Object.keys(OUTCOME), ...new Set(findings.map(f => f.outcome).filter(o => !OUTCOME[o]))];
    h += `<div class="panel"${expandAttrs('findings', g)}>
      ${expandBtnHtml('指摘')}
      <h3><span class="material-symbols-outlined" style="font-size:18px;">rule</span><span>指摘 ${findings.length}件</span></h3>
      <div class="body">` +
      groups.map(outcome => {
        const mine = findings.filter(f => f.outcome === outcome);
        if (!mine.length) return '';
        return `<h5>${esc(OUTCOME[outcome]?.[0] || outcome)} ${mine.length}件</h5><ul style="padding-left:20px;line-height:1.7;">` + mine.map(f => {
          const openMust = f.outcome === 'open' && f.severity === 'must';
          return `<li${openMust ? ' style="color:var(--md-sys-color-error);font-weight:600;"' : ''}>` +
            `<span class="m3-pill ${f.severity === 'must' ? 'pill-critical' : 'pill-gray'}" style="font-size:10px;padding:1px 6px;margin-right:6px;">${esc(f.severity)}</span>` +
            `${f.location ? `<code style="font-family:var(--font-mono);font-size:11px;background:var(--md-sys-color-surface-container-high);padding:2px 6px;border-radius:4px;margin-right:6px;">${esc(f.location)}</code> ` : ''}${esc(f.text)}` +
            `${f.reason ? `<br><span style="color:var(--md-sys-color-outline);font-size:12px;">${f.outcome === 'declined' ? '却下の理由' : '理由'}: ${esc(f.reason)}</span>` : ''}</li>`;
        }).join('') + '</ul>';
      }).join('') + `</div></div>`;
  }
  return h;
}

function checkPanels(g) {
  let h = '';
  const commands = g.commands || [];
  if (commands.length) {
    h += `<div class="panel"${expandAttrs('commands', g)}>
      ${expandBtnHtml('Verify 実行結果')}
      <h3><span class="material-symbols-outlined" style="font-size:18px;">fact_check</span><span>Verify 実行結果</span></h3>` +
      commands.map(c => {
        const failed = c.result === 'fail';
        const retried = !failed && (c.attempts || 1) > 1;
        const head = `<span class="state ${failed ? 'bad' : 'good'}" style="display:inline-flex;align-items:center;gap:6px;">` +
          `<span class="material-symbols-outlined" style="font-size:16px;color:${failed ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-success)'};">${failed ? 'cancel' : 'check_circle'}</span>` +
          `<code style="font-family:var(--font-mono);font-weight:600;">${esc(c.command)}</code></span>` +
          (retried ? ` <span class="m3-pill pill-warn" style="font-size:11px;" title="1回目は失敗して、直してから通った">${c.attempts}回目で通過</span>` : '') +
          (c.time ? ` <span style="color:var(--md-sys-color-outline);font-size:12px;margin-left:auto;">${esc(c.time)}</span>` : '');
        return c.output
          ? `<details${failed ? ' open' : ''} style="margin-bottom:8px;background:var(--md-sys-color-surface-container-low);padding:8px 12px;border-radius:var(--md-shape-corner-sm);"><summary style="cursor:pointer;list-style:none;display:flex;align-items:center;">${head}</summary>` +
            `<div class="diff" style="margin-top:8px;"><div>${esc(c.output).split('\n').join('</div><div>')}</div></div></details>`
          : `<div style="margin-bottom:8px;background:var(--md-sys-color-surface-container-low);padding:8px 12px;border-radius:var(--md-shape-corner-sm);display:flex;align-items:center;">${head}</div>`;
      }).join('') + `</div>`;
  }
  if ((g.manual || []).length) {
    h += `<div class="panel"${expandAttrs('manual', g)}>
      ${expandBtnHtml('人が見る確認項目')}
      <h3><span class="material-symbols-outlined" style="font-size:18px;">visibility</span><span>人が見る確認項目</span></h3>
      <ul class="checklist" style="list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px;">${g.manual.map((m, i) =>
        `<li><label style="display:flex;align-items:flex-start;gap:8px;font-size:13px;cursor:pointer;"><input type="checkbox" data-manual-gate="${esc(gateRef(g))}" data-manual-index="${i}"${manualChecked(gateRef(g)).has(i) ? ' checked' : ''} style="margin-top:3px;cursor:pointer;"><span>${esc(m)}</span></label></li>`).join('')}</ul></div>`;
  }
  return h;
}

/* Ticks on a gate's manual checks, kept per gate so a re-render or a tab switch does not
   drop them. */
const manualChecks = new Map();
function manualChecked(gateId) {
  if (!manualChecks.has(gateId)) manualChecks.set(gateId, new Set());
  return manualChecks.get(gateId);
}
document.addEventListener('change', e => {
  const box = e.target.closest?.('input[data-manual-gate]');
  if (!box) return;
  const ticked = manualChecked(box.dataset.manualGate);
  const i = Number(box.dataset.manualIndex);
  if (box.checked) ticked.add(i); else ticked.delete(i);
});

/* The comment box of the panel a gate is judged in. */
const commentBox = () => document.querySelector('#card-dialog[open] .gate-comment') || document.querySelector('#task-panel .gate-comment');

/* An answered gate leaves the list and the counts at once; the round that follows confirms it.
   In a merged state that round can be a while off. */
function dropGate(g) {
  const listed = (state.gates || []).some(x => x.id === g.id && x._slug === g._slug);
  state = { ...state, gates: (state.gates || []).filter(x => !(x.id === g.id && x._slug === g._slug)) };
  // The sidebar's rows and the badge count what /api/boards lists: the next answer corrects them.
  const row = g._slug && boards.find(b => b.slug === g._slug);
  if (row && (row.gates || []).some(x => x.id === g.id)) {
    row.gates = row.gates.filter(x => x.id !== g.id);
    row.waiting = Math.max(0, (row.waiting || 0) - 1);
  }
  if (listed || row) render();
}

async function answer(decision, choice, id, commentOverride = null) {
  const g = gateByRef(id);
  if (!g) return false;
  const box = commentBox();
  const comment = commentOverride !== null ? commentOverride : box?.value.trim();
  // Sending back something the worker has moved past, without saying what is wrong, gives it
  // nothing to act on.
  if (g.wait === false && !comment) {
    note(`adj gate answer --id ${g.id} --decision changes`, true, '差し戻す理由をコメントに書いてください');
    return false;
  }
  const line = `adj gate answer --id ${g.id} --decision ${decision}` + (choice ? ` --choice ${choice}` : '');
  const key = gateKey(g);
  try {
    const data = await boardApi(baseOf(g), `/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision, choice, comment }),
    });
    // The hub opened this gate, and its answer goes to the hub's inbox instead.
    const toHub = g.answeredByHub;
    note(line, false, toHub
      ? 'hub の受信箱に送信' + handedNote({ present: data.present, woken: data.woken })
      : `${baseName(g.worktree)} の outbox に追記` +
        (data.woken ? ' → worker に通知しました' : data.present ? ' → worker は次回の outbox 確認時に読み込みます'
                                                               : ' → worker は停止中のため、回答は outbox で保持されます'));
    // A record stays, with this answer appended, so it stays in view to show that it went.
    if (g.wait === false && box) box.value = '';
    else gateAnswered(g, key);
    dropGate(g);
    await refresh(true);
    refreshBoards();
    return true;
  } catch (e) {
    note(line, true, e.message);
    return false;
  }
}

/* The escape hatch from "見せて決める" to "話して決める". The gate stays open on purpose:
   the ball is still with the human until they come back and close it. */
function talk(id) {
  // The work view has the worker's terminal in the middle, already open: the focus goes to it, and
  // the panel's tab stays.
  if (view === 'work') return focusWorkTerm();
  const g = gateByRef(id);
  if (!g) return;
  // A gate the hub opened sits in the main checkout, where there is no worker: its tab is the
  // hub's.
  if (g.answeredByHub) focusHub(g._slug); else worktreeAct('focus', g.worktree, false, g._slug);
  note('gate は開いたままです', false, 'タブで確認後、「解決済みとして閉じる」を押してください');
}

async function closeGate(id) {
  const g = gateByRef(id);
  if (!g) return false;
  const comment = commentBox()?.value.trim();
  const line = `adj gate close --id ${g.id}` + (comment ? ` --comment '${comment}'` : '');
  const key = gateKey(g);
  try {
    await boardApi(baseOf(g), `/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision: 'close', comment: comment || 'タブで解決済み' }),
    });
    note(line, false, '解決済みとしてアーカイブしました（worker への outbox 配信なし）');
    gateAnswered(g, key);
    dropGate(g);
    await refresh(true);
    refreshBoards();
    return true;
  } catch (e) { note(line, true, e.message); return false; }
}
