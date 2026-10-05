/* Each button names its decision in `data-act`; `bindDecide` finds the gate it belongs to from
   the `data-gate` around it, so the same panel works in the review view and a task's view. */
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
      <button class="btn-m3-text close" style="margin-left:auto;color:var(--md-sys-color-outline)" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします">
        <span class="material-symbols-outlined" style="font-size:16px;">done_all</span>
        <span>解決済みとして閉じる</span>
      </button>
    </div>
    <div style="font-size:11.5px;color:var(--md-sys-color-outline);display:flex;align-items:flex-start;gap:6px;margin-top:8px;">
      <span class="material-symbols-outlined" style="font-size:14px;margin-top:2px;">info</span>
      <span>
        ${hubsGate(g)
          ? '判定は <code>adj gate answer</code> → hub の受信箱に送信 → <code>hubWake</code> で hub に通知します。'
          : '判定は <code>adj gate answer</code> → 対象 worktree の outbox に追記 → <code>workerWake</code> で worker に通知します。'}<br>
        <b>「ターミナルで話す」</b>は gate を開いたまま${hubsGate(g) ? ' hub' : ' worker'} タブを前面表示します。直接確認した後は<b>「解決済みとして閉じる」</b>を押してください（${hubsGate(g) ? 'hub への配信' : 'worker への outbox 配信'}なしでアーカイブします）。
      </span>
    </div>
  </div>`;
}

/* The buttons `decideHtml` and a gate's choices draw, wired to the gate they sit in. */
function bindDecide(root) {
  root.querySelectorAll('[data-gate] [data-act]').forEach(b => b.addEventListener('click', () => {
    const id = b.closest('[data-gate]').dataset.gate;
    const act = b.dataset.act;
    if (act === 'talk') talk(id);
    else if (act === 'close') closeGate(id);
    else answer(act, undefined, id);
  }));
  root.querySelectorAll('[data-gate] .pick[data-choice]').forEach(b =>
    b.addEventListener('click', () => answer('choice', b.dataset.choice, b.closest('[data-gate]').dataset.gate)));
  root.querySelectorAll('button[data-ide]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('ide', b.dataset.ide)));
  root.querySelectorAll('button[data-focus]').forEach(b =>
    b.addEventListener('click', () => worktreeAct('focus', b.dataset.focus)));
}

/* A gate's designs side by side. Only an open gate can still be answered with one; once
   answered, the one picked is marked instead. */
function choicesHtml(g, pickable) {
  if (!g.choices?.length) return '';
  return `<div class="panel"${pickable ? ` data-gate="${esc(gateRef(g))}"` : ''}>
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
    h += `<div class="panel">
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
    h += `<div class="panel">
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
    h += `<div class="panel">
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
    h += `<div class="panel">
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

/* The comment box of the view on screen. The review view and a task's view can both hold one
   at once, the hidden one included, so it is looked up inside the one being shown. */
const commentBox = () => document.querySelector(view === 'task' ? '#task-view .gate-comment' : '#review .gate-comment');

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

async function answer(decision, choice, id = focused, commentOverride = null) {
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
  try {
    const data = await boardApi(baseOf(g), `/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision, choice, comment }),
    });
    // The hub opened this gate, and its answer goes to the hub's inbox instead.
    const toHub = hubsGate(g);
    note(line, false, toHub
      ? 'hub の受信箱に送信' + handedNote({ present: data.present, woken: data.woken })
      : `${baseName(g.worktree)} の outbox に追記` +
        (data.woken ? ' → worker に通知しました' : data.present ? ' → worker は次回の outbox 確認時に読み込みます'
                                                               : ' → worker は停止中のため、回答は outbox で保持されます'));
    // A record stays, with this answer appended, so it stays in view to show that it went.
    if (g.wait === false && box) box.value = '';
    else {
      reviewAnswered(g, decision);
      if (view !== 'review' && focused === gateRef(g)) focused = null;
    }
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
function talk(id = focused) {
  const g = gateByRef(id);
  if (!g) return;
  // A gate the hub opened sits in the main checkout, where there is no worker: its tab is the
  // hub's.
  if (hubsGate(g)) focusHub(g._slug); else worktreeAct('focus', g.worktree, false, g._slug);
  note('gate は開いたままです', false, 'タブで確認後、「解決済みとして閉じる」を押してください');
}

async function closeGate(id = focused) {
  const g = gateByRef(id);
  if (!g) return;
  const comment = commentBox()?.value.trim();
  const line = `adj gate close --id ${g.id}` + (comment ? ` --comment '${comment}'` : '');
  try {
    await boardApi(baseOf(g), `/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision: 'close', comment: comment || 'タブで解決済み' }),
    });
    note(line, false, '解決済みとしてアーカイブしました（worker への outbox 配信なし）');
    reviewAnswered(g, 'close');
    if (view !== 'review' && focused === gateRef(g)) focused = null;
    dropGate(g);
    await refresh(true);
    refreshBoards();
  } catch (e) { note(line, true, e.message); }
}
