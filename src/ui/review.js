// ── the review view ───────────────────────────────────────────────────

/* Gate kinds take categorical slots in a fixed order, never cycled. Each one ships with a
   label beside the dot: the colour is a landmark, never the carrier. */
const KINDS = {
  plan:     ['設計レビュー',  '#2a78d6'],
  diff:     ['コードレビュー','#eb6834'],
  verify:   ['動作確認',      '#1baf7a'],
  result:   ['調査報告',      '#e87ba4'],
  dispatch: ['着手確認',      '#4a3aa7'],
  issue:    ['起票の確認',    '#eda100'],
  question: ['質問',          '#52514e'],
};
const kindOf = k => KINDS[k] || [k, '#898781'];

let focused = null;

/* `20260922T041233Z` → 「4分前」. The stamp is UTC and says so; the reader wants neither. */
function ago(stamp) {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp || '');
  if (!m) return stamp || '';
  const then = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  const mins = Math.max(0, Math.round((Date.now() - then) / 60000));
  if (mins < 1) return 'たった今';
  if (mins < 60) return `${mins}分前`;
  const hours = Math.round(mins / 60);
  return hours < 24 ? `${hours}時間前` : `${Math.round(hours / 24)}日前`;
}

/* Render markdown into safe HTML: headings (H1-H5), code blocks, tables, lists, blockquotes,
   horizontal rules, links, and inline decorations (bold, italic, strikethrough, inline code).
   Escaping happens before parsing so untrusted HTML is never executed. */
function md(src) {
  if (!src) return '';
  const lines = esc(src).split('\n');
  let html = '';
  let inCode = false;
  let codeLang = '';
  let codeContent = [];
  let inTable = false;
  let tableRows = [];
  let listStack = [];
  let currentPara = [];

  function closePara() {
    if (currentPara.length) {
      html += `<p>${currentPara.join('<br>')}</p>`;
      currentPara = [];
    }
  }

  function closeLists() {
    closePara();
    while (listStack.length) {
      html += `</${listStack.pop()}>`;
    }
  }

  function closeTable() {
    if (!inTable) return;
    inTable = false;
    if (tableRows.length === 0) return;
    html += '<table>';
    let startIdx = 0;
    if (tableRows.length >= 2 && tableRows[1].every(cell => /^:?-+:?$/.test(cell.trim()))) {
      html += '<thead><tr>' + tableRows[0].map(c => `<th>${inlineMd(c.trim())}</th>`).join('') + '</tr></thead>';
      startIdx = 2;
    }
    if (startIdx < tableRows.length) {
      html += '<tbody>';
      for (let i = startIdx; i < tableRows.length; i++) {
        html += '<tr>' + tableRows[i].map(c => `<td>${inlineMd(c.trim())}</td>`).join('') + '</tr>';
      }
      html += '</tbody>';
    }
    html += '</table>';
    tableRows = [];
  }

  function closeAll() {
    closePara();
    closeLists();
    closeTable();
  }

  function inlineMd(text) {
    return text
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
      .replace(/\*([^*]+)\*/g, '<i>$1</i>')
      .replace(/~~([^~]+)~~/g, '<del>$1</del>')
      .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  }

  for (let idx = 0; idx < lines.length; idx++) {
    const raw = lines[idx];

    // Fenced code blocks
    const codeMatch = raw.match(/^```(\w*)/);
    if (codeMatch && !inCode) {
      closeAll();
      inCode = true;
      codeLang = codeMatch[1] || '';
      codeContent = [];
      continue;
    } else if (raw.startsWith('```') && inCode) {
      inCode = false;
      html += `<pre><code class="${codeLang}">${codeContent.join('\n')}</code></pre>`;
      continue;
    }
    if (inCode) {
      codeContent.push(raw);
      continue;
    }

    // Table rows
    const isTableRow = raw.trim().startsWith('|') && raw.trim().endsWith('|');
    if (isTableRow) {
      if (!inTable) {
        closeAll();
        inTable = true;
      }
      const cells = raw.trim().slice(1, -1).split('|');
      tableRows.push(cells);
      continue;
    } else if (inTable) {
      closeTable();
    }

    // Headings
    let m;
    if ((m = raw.match(/^####\s+(.*)$/))) {
      closeAll();
      html += `<h5>${inlineMd(m[1])}</h5>`;
      continue;
    }
    if ((m = raw.match(/^###\s+(.*)$/))) {
      closeAll();
      html += `<h5>${inlineMd(m[1])}</h5>`;
      continue;
    }
    if ((m = raw.match(/^##\s+(.*)$/))) {
      closeAll();
      html += `<h4>${inlineMd(m[1])}</h4>`;
      continue;
    }
    if ((m = raw.match(/^#\s+(.*)$/))) {
      closeAll();
      html += `<h3>${inlineMd(m[1])}</h3>`;
      continue;
    }

    // Horizontal rule
    if (/^(\*\*\*|---|___)$/.test(raw.trim())) {
      closeAll();
      html += '<hr>';
      continue;
    }

    // Blockquote
    if ((m = raw.match(/^(?:&gt;|>)\s*(.*)$/))) {
      closeAll();
      html += `<blockquote><p>${inlineMd(m[1])}</p></blockquote>`;
      continue;
    }

    // Bullet list
    if ((m = raw.match(/^(\s*)([-*+])\s+(.*)$/))) {
      closePara();
      closeTable();
      if (!listStack.length || listStack[listStack.length - 1] !== 'ul') {
        closeLists();
        listStack.push('ul');
        html += '<ul>';
      }
      html += `<li>${inlineMd(m[3])}</li>`;
      continue;
    }

    // Numbered list
    if ((m = raw.match(/^(\s*)(\d+)\.\s+(.*)$/))) {
      closePara();
      closeTable();
      if (!listStack.length || listStack[listStack.length - 1] !== 'ol') {
        closeLists();
        listStack.push('ol');
        html += '<ol>';
      }
      html += `<li>${inlineMd(m[3])}</li>`;
      continue;
    }

    // Blank line terminates paragraphs/blocks
    if (!raw.trim()) {
      closeAll();
      continue;
    }

    // Paragraph text line
    currentPara.push(inlineMd(raw));
  }

  closeAll();
  if (inCode) {
    html += `<pre><code>${codeContent.join('\n')}</code></pre>`;
  }
  return html;
}


const renderDiff = d => esc(d).split('\n').map(l => {
  const cls = l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@') ? 'h'
            : l.startsWith('+') ? 'a' : l.startsWith('-') ? 'd' : '';
  return `<div class="${cls}">${l || ' '}</div>`;
}).join('');

/* Each button names its decision in `data-act`; `bindDecide` finds the gate it belongs to from
   the `data-gate` around it, so the same panel works in the review view and a task's view. */
const BUTTONS = {
  approve: g => `<button class="btn-m3-primary approve" data-act="approve" style="background:var(--md-sys-color-success);color:var(--md-sys-color-on-success)"><span class="material-symbols-outlined" style="font-size:16px;">check</span><span>${g.kind === 'verify' ? 'OK — 受け入れる' : '承認' + (g.kind === 'diff' ? 'して PR へ' : '')}</span></button>`,
  changes: g => `<button class="btn-m3-tonal changes" data-act="changes"><span class="material-symbols-outlined" style="font-size:16px;">replay</span><span>${g.kind === 'verify' ? 'NG — 直してほしい' : '修正を指示'}</span></button>`,
  reject:  () => `<button class="btn-m3-text reject" data-act="reject" style="color:var(--md-sys-color-error);"><span class="material-symbols-outlined" style="font-size:16px;">cancel</span><span>却下</span></button>`,
  ack:     () => `<button class="btn-m3-primary approve" data-act="ack"><span class="material-symbols-outlined" style="font-size:16px;">check</span><span>了解(閉じる)</span></button>`,
  ask:     () => `<button class="btn-m3-tonal changes" data-act="ask"><span class="material-symbols-outlined" style="font-size:16px;">help</span><span>追加で聞く</span></button>`,
  answer:  () => `<button class="btn-m3-primary approve" data-act="answer"><span class="material-symbols-outlined" style="font-size:16px;">send</span><span>これで返す</span></button>`,
};

/* The part of a gate a person acts on: the send-back form for a record, the decision for a
   gate that waits. */
function decideHtml(g) {
  if (g.wait === false) {
    return `<div class="decision-dock panel" data-gate="${esc(g.id)}">
      <h3 style="font-size:14px;font-weight:800;display:flex;align-items:center;gap:6px;color:var(--md-sys-color-on-surface);">
        <span class="material-symbols-outlined" style="font-size:18px;color:var(--md-sys-color-primary);">replay</span>
        <span>差し戻す</span>
      </h3>` +
      ((g.answers || []).length ? `<ul style="margin:0 0 12px;padding-left:18px">${g.answers.map(a =>
        `<li><span style="color:var(--md-sys-color-outline)">${ago(a.answeredAt)}に差し戻し</span>${a.comment ? ` — ${esc(a.comment)}` : ''}</li>`).join('')}</ul>` : '') +
      `<div class="field" style="margin-bottom:12px">
        <label style="font-size:12px;font-weight:600;color:var(--md-sys-color-on-surface-variant);margin-bottom:4px;display:block;">コメント（何を直してほしいか）</label>
        <textarea class="gate-comment" placeholder="差し戻す理由を入力してください..."></textarea>
      </div>
      <div class="decide">
        <button class="btn-m3-tonal changes" data-act="changes">
          <span class="material-symbols-outlined" style="font-size:16px;">replay</span>
          <span>差し戻す</span>
        </button>
        ${g.worktree ? `
          <button class="m3-icon-button" style="padding:8px 14px" data-focus="${esc(g.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">terminal</span>
            <span>ターミナルで話す</span>
          </button>
          <button class="m3-icon-button" style="padding:8px 14px" data-ide="${esc(g.worktree)}">
            <span class="material-symbols-outlined" style="font-size:16px;">code</span>
            <span>IDEで開く</span>
          </button>
        ` : ''}
      </div>
      <p style="color:var(--md-sys-color-outline);font-size:11.5px;margin-top:10px">
        worker はこの記録を残して先に進んでいます。差し戻すと <code>adj gate answer</code> → 対象 worktree の outbox に追記 → <code>workerWake</code> で worker に通知します。記録はそのまま残り、差し戻しが追記されます。</p></div>`;
  }
  return `<div class="decision-dock panel" data-gate="${esc(g.id)}">
    <h3 style="font-size:14px;font-weight:800;display:flex;align-items:center;gap:6px;color:var(--md-sys-color-on-surface);">
      <span class="material-symbols-outlined" style="font-size:18px;color:var(--md-sys-color-primary);">gavel</span>
      <span>${g.kind === 'verify' ? '確かめたら（判定）' : 'あなたの判定・指示'}</span>
    </h3>` +
    ((g.rounds || 0) >= 2 ? `<div class="hint-bar" style="display:flex;align-items:center;gap:6px;margin-bottom:12px;">
      <span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">warning</span>
      <span>ここまで ${g.rounds} 往復しています。<b>ターミナルで直接やり取りしたほうが円滑</b>です（1往復ごとに outbox と wake を経由します）</span>
    </div>` : '') +
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
        <button class="m3-icon-button" style="padding:8px 14px" data-ide="${esc(g.worktree)}">
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
        ${['dispatch', 'issue'].includes(g.kind)
          ? '判定は <code>adj gate answer</code> → hub の受信箱に送信 → <code>hubWake</code> で hub に通知します。'
          : '判定は <code>adj gate answer</code> → 対象 worktree の outbox に追記 → <code>workerWake</code> で worker に通知します。'}<br>
        <b>「ターミナルで話す」</b>は gate を開いたまま worker タブを前面表示します。直接確認した後は<b>「解決済みとして閉じる」</b>を押してください（worker への outbox 配信なしでアーカイブします）。
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

let reviewActiveTab = 'overview';
function switchReviewTab(tabName) {
  reviewActiveTab = tabName;
  renderReview();
  // The tab buttons were just replaced, so keyboard focus goes back to the selected one.
  document.querySelector('#review .m3-seg-tab.active')?.focus({ preventScroll: true });
}
window.switchReviewTab = switchReviewTab;

/* A redraw that came while a comment was being typed in the review view, held until the box
   is left, as the task view does: replacing the markup under the box cuts an IME composition
   short even when the text is put back. A person's own action still redraws at once. */
let reviewHeld = false;
function redrawReview() {
  if (document.activeElement?.matches('#review .gate-comment')) {
    reviewHeld = true;
    return;
  }
  renderReview();
}
document.getElementById('review').addEventListener('focusout', e => {
  if (!reviewHeld || !e.target.matches('.gate-comment')) return;
  // After the focus has moved: a click on a button redraws through its own handler, and this
  // one should not draw over it with the old state.
  setTimeout(() => {
    if (!reviewHeld || document.activeElement?.matches('#review .gate-comment')) return;
    renderReview();
  });
});

function renderReview() {
  reviewHeld = false;
  const rail = document.querySelector('#review .rail');
  const pane = document.querySelector('#review .pane');
  const gates = state.gates || [];

  rail.className = 'review-inbox-pane rail';
  pane.className = 'review-detail-pane pane';

  rail.innerHTML = `
    <div class="review-inbox-header">
      <div style="display:flex;align-items:center;gap:6px;">
        <span class="material-symbols-outlined" style="font-size:18px;color:var(--md-sys-color-primary);">inbox</span>
        <span>要対応 (${gates.length})</span>
      </div>
      <div style="font-size:11px;font-weight:normal;color:var(--md-sys-color-outline)">
        <kbd>j</kbd><kbd>k</kbd> 移動 &nbsp;<kbd>a</kbd> 承認 &nbsp;<kbd>r</kbd> 修正 &nbsp;<kbd>c</kbd> 閉じる
      </div>
    </div>
    <div class="review-inbox-list"></div>
  `;
  const listEl = rail.querySelector('.review-inbox-list');

  for (const g of gates) {
    const isCur = (g.id === focused);
    const [label, colour] = kindOf(g.kind);
    const b = document.createElement('button');
    b.className = 'review-inbox-item item' + (isCur ? ' selected' : '');
    b.setAttribute('aria-current', String(isCur));
    b.innerHTML = `
      <div style="display:flex;align-items:center;gap:6px">
        <span class="m3-pill ${g.kind === 'plan' ? 'pill-blue' : g.kind === 'diff' ? 'pill-purple' : 'pill-warn'}">${label}</span>
        <span class="w" style="font-size:11px;color:var(--md-sys-color-outline);margin-left:auto;">${ago(g.openedAt)}</span>
      </div>
      <div class="t" style="font-weight:700;font-size:13px;line-height:1.35;">${esc(g.title)}</div>
      <div style="font-size:11px;color:var(--md-sys-color-outline);font-family:var(--font-mono);">${esc(g.worktree ? g.worktree.split('/').pop() : '')}</div>
    `;
    b.onclick = () => {
      focused = g.id;
      reviewActiveTab = TAB_OF_KIND[g.kind] || 'overview';
      renderReview();
    };
    listEl.appendChild(b);
  }

  // A record is shown here too, opened from a card or the drawer. It is not in the rail, which
  // is what waits on a person, and a record does not.
  const g = gates.find(x => x.id === focused) || recordById(focused) || gates[0];
  if (!g) {
    pane.innerHTML = `
      <div class="empty-state" style="padding:80px 20px;text-align:center;">
        <span class="material-symbols-outlined" style="font-size:48px;color:var(--md-sys-color-outline);opacity:.5;margin-bottom:12px;display:block;">done_all</span>
        <div style="font-size:15px;font-weight:700;color:var(--md-sys-color-on-surface);">現在、対応待ちの判定はありません</div>
        <div style="font-size:13px;color:var(--md-sys-color-outline);margin-top:6px;">ボード画面でお待ちください</div>
      </div>
    `;
    if (view === 'review') location.hash = '';
    return;
  }
  focused = g.id;
  const record = g.wait === false;
  if (record && view === 'review') markSeen(g.id);
  // A permalink, so "反映しといたから見てね" can point at this one card.
  if (view === 'review') location.hash = 'gate/' + g.id;

  if (pane.dataset.shown !== g.id) {
    reviewActiveTab = TAB_OF_KIND[g.kind] || 'overview';
  }

  const wasDecidedOpen = pane.querySelector('details.decided')?.open;
  const commentEl = pane.querySelector('.gate-comment');
  const commentVal = commentEl && pane.dataset.shown === g.id ? commentEl.value : '';
  const isCommentFocused = document.activeElement === commentEl;
  const paneScroll = pane.scrollTop;

  const parentTask = (state.tasks || []).find(t => t.id === g.task) || (state.tasks || []).find(t => t.worktree && t.worktree === g.worktree);
  const taskTitle = parentTask ? parentTask.title : g.title;
  const taskId = parentTask ? parentTask.id : (g.task || g.id);
  const taskBranch = parentTask ? parentTask.branch : '';
  const taskWorktree = parentTask ? parentTask.worktree : g.worktree;
  const taskDoneWhen = parentTask ? (DONE_WHEN[parentTask.doneWhen] || parentTask.doneWhen) : '—';
  const taskIssue = httpUrl(parentTask?.issueUrl);

  const gateTargetTab = TAB_OF_KIND[g.kind] || null;
  const [gateLabel, gateColour] = kindOf(g.kind);

  let h = '';

  // ── 1. 対象タスクの基本ヘッダー ──
  h += `
    <div style="display:flex;flex-direction:column;gap:6px;">
      <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;">
        ${parentTask ? `<span class="m3-pill pill-blue">${esc(columnOf(parentTask))}</span>` : ''}
        <span style="font-family:var(--font-mono);font-size:12px;color:var(--md-sys-color-outline);">${esc(taskId)}</span>
        ${taskIssue ? `<a href="${esc(taskIssue)}" target="_blank" rel="noopener noreferrer" style="margin-left:auto;font-size:12px;color:var(--md-sys-color-primary);text-decoration:none;font-weight:600;display:inline-flex;align-items:center;gap:4px;"><span>Issue #${esc(issueNumberOf(taskIssue))}</span><span class="material-symbols-outlined" style="font-size:14px;">open_in_new</span></a>` : ''}
      </div>
      <h1 style="font-size:22px;font-weight:800;color:var(--md-sys-color-on-surface);margin:2px 0 0;line-height:1.3;">${esc(taskTitle)}</h1>
      <div style="font-size:12px;color:var(--md-sys-color-on-surface-variant);display:flex;gap:14px;flex-wrap:wrap;align-items:center;">
        <span>ブランチ: <code style="font-family:var(--font-mono);font-size:12px;">${esc(taskBranch || '—')}</code></span>
        <span>worktree: <code style="font-family:var(--font-mono);font-size:12px;">${esc(taskWorktree ? taskWorktree.split('/').pop() : '—')}</code></span>
        <span>完了条件: <strong>${esc(taskDoneWhen)}</strong></span>
        ${taskWorktree ? `
          <span style="display:inline-flex;gap:6px;margin-left:auto;">
            <button class="m3-icon-button" style="padding:4px 10px;font-size:11px;" title="ターミナルのworkerタブを前面表示" data-focus="${esc(taskWorktree)}">
              <span class="material-symbols-outlined" style="font-size:14px;">terminal</span>
              <span>端末</span>
            </button>
            <button class="m3-icon-button" style="padding:4px 10px;font-size:11px;" title="IDEでworktreeを開く" data-ide="${esc(taskWorktree)}">
              <span class="material-symbols-outlined" style="font-size:14px;">code</span>
              <span>IDE</span>
            </button>
          </span>
        ` : ''}
      </div>
    </div>
  `;

  // ── 2. 現在の確認待ち（Gate）アラートバナー ──
  h += `
    <div class="m3-card-attention-box" style="padding:14px 18px;display:flex;flex-direction:column;gap:8px;">
      <div style="display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:8px;">
        <div style="font-weight:800;font-size:14px;display:flex;align-items:center;gap:6px;">
          <span class="material-symbols-outlined" style="font-size:18px;">${record ? 'history' : 'pending_actions'}</span>
          <span>【${esc(gateLabel)}】${record ? '記録 — worker は止まらずに進んだ' : 'あなたの判定待ち'}</span>
        </div>
        <span style="font-size:11.5px;opacity:.9">${ago(g.openedAt)}${record ? 'に記録' : 'から待ち'} (ID: ${esc(g.id)})</span>
      </div>
  `;
  if (stopWhy(g).length) {
    h += `
      <div style="font-size:12.5px;">
        <strong>止めた理由:</strong>
        <ul style="margin:4px 0 0;padding-left:18px;">${stopWhy(g).map(w =>
          `<li${stopBad(g) ? ' style="color:var(--md-sys-color-error)"' : ''}>${esc(w)}</li>`).join('')}</ul>
      </div>
    `;
  }
  if (g.focus) {
    h += `
      <div style="font-size:12.5px;background:rgba(0,0,0,.04);padding:8px 12px;border-radius:var(--md-shape-corner-xs);">
        <strong>確認してほしい点:</strong> ${md(g.focus)}
      </div>
    `;
  }
  if (gateTargetTab && reviewActiveTab !== gateTargetTab) {
    const targetTabLabel = gateTargetTab === 'overview' ? '概要・計画' : gateTargetTab === 'review' ? 'コードレビュー' : '動作確認';
    h += `
      <div style="margin-top:2px;">
        <button class="btn-m3-tonal" style="padding:4px 12px;font-size:12px;border-radius:var(--md-shape-corner-full);" onclick="switchReviewTab('${gateTargetTab}')">
          <span class="material-symbols-outlined" style="font-size:14px;">arrow_forward</span>
          <span>判定対象タブ（${targetTabLabel}）を見る</span>
        </button>
      </div>
    `;
  }
  h += `</div>`;

  // ── 3. M3 Segmented Tabs (4つのタブ) ──
  h += `
    <div class="m3-segmented-tabs">
      <button class="m3-seg-tab ${reviewActiveTab === 'overview' ? 'active' : ''}" aria-pressed="${reviewActiveTab === 'overview'}" onclick="switchReviewTab('overview')">
        <span class="material-symbols-outlined">description</span>
        <span>概要・計画</span>
        ${gateTargetTab === 'overview' && !record ? '<span class="m3-tab-badge">要判定</span>' : ''}
      </button>
      <button class="m3-seg-tab ${reviewActiveTab === 'review' ? 'active' : ''}" aria-pressed="${reviewActiveTab === 'review'}" onclick="switchReviewTab('review')">
        <span class="material-symbols-outlined">rate_review</span>
        <span>コードレビュー</span>
        ${gateTargetTab === 'review' && !record ? '<span class="m3-tab-badge">要判定</span>' : ''}
      </button>
      <button class="m3-seg-tab ${reviewActiveTab === 'check' ? 'active' : ''}" aria-pressed="${reviewActiveTab === 'check'}" onclick="switchReviewTab('check')">
        <span class="material-symbols-outlined">fact_check</span>
        <span>動作確認 (Verify)</span>
        ${gateTargetTab === 'check' && !record ? '<span class="m3-tab-badge">要判定</span>' : ''}
      </button>
      <button class="m3-seg-tab ${reviewActiveTab === 'history' ? 'active' : ''}" aria-pressed="${reviewActiveTab === 'history'}" onclick="switchReviewTab('history')">
        <span class="material-symbols-outlined">history</span>
        <span>タイムライン・全履歴</span>
      </button>
    </div>
  `;

  // ── 4. タブ別コンテンツ領域 ──
  h += `<div style="display:flex;flex-direction:column;gap:18px;">`;

  if (reviewActiveTab === 'overview') {
    if (parentTask?.body) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">assignment</span><span>依頼内容 / 要件プロンプト</span></h3>
        <div class="body">${md(parentTask.body)}</div>
      </div>`;
    }
    if (g.body && g.body !== parentTask?.body) {
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
    h += choicesHtml(g, !record);
  } else if (reviewActiveTab === 'review') {
    h += reviewPanels(g);
    if (g.diff) {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">difference</span><span>コード差分 (Diff)</span></h3>
        <div class="diff">${renderDiff(g.diff)}</div>
      </div>`;
    } else if (!g.findings?.length && !g.reviewRounds?.length) {
      h += `<div class="empty-state" style="padding:40px;text-align:center;">この Gate に記録されたコード差分はありません。</div>`;
    }
  } else if (reviewActiveTab === 'check') {
    if (g.kind === 'verify' && !record) {
      h += `<div class="work">
        <button class="big" data-ide="${esc(g.worktree)}">
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
  } else if (reviewActiveTab === 'history') {
    if (g.decided) {
      h += `<div class="panel"><details class="decided"${wasDecidedOpen ? ' open' : ''}><summary style="font-weight:700;cursor:pointer;">決定事項</summary><div class="body" style="margin-top:8px;">${md(g.decided)}</div></details></div>`;
    }
    if (parentTask) {
      h += historyTab(parentTask, gatesOf(parentTask));
    } else {
      h += `<div class="panel">
        <h3><span class="material-symbols-outlined" style="font-size:18px;">history</span><span>Gate 履歴</span></h3>
        <div style="font-size:13px;line-height:1.8;">
          <div><strong style="color:var(--md-sys-color-outline);">${when(g.openedAt)}:</strong> 【${esc(gateLabel)}】Gate をオープン — 人間の判定待ち</div>
          ${(g.answers || []).map(a => `<div><strong style="color:var(--md-sys-color-outline);">${when(a.answeredAt)}:</strong> 人が差し戻し — ${esc(a.comment || '')}</div>`).join('')}
        </div>
      </div>`;
    }
  }

  h += `</div>`;

  // ── 5. Decision Dock (最下部判定パネル) ──
  h += decideHtml(g);

  pane.innerHTML = h;
  pane.dataset.shown = g.id;
  bindDecide(pane);
  pane.querySelectorAll('[data-open]').forEach(b => b.addEventListener('click', () => {
    focused = b.dataset.open;
    renderReview();
  }));
  restoreComment(pane, commentVal, isCommentFocused, paneScroll);
}

/* A gate's designs side by side. Only an open gate can still be answered with one; once
   answered, the one picked is marked instead. */
function choicesHtml(g, pickable) {
  if (!g.choices?.length) return '';
  return `<div class="panel"${pickable ? ` data-gate="${esc(g.id)}"` : ''}>
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
        `<li><label style="display:flex;align-items:flex-start;gap:8px;font-size:13px;cursor:pointer;"><input type="checkbox" data-manual-gate="${esc(g.id)}" data-manual-index="${i}"${manualChecked(g.id).has(i) ? ' checked' : ''} style="margin-top:3px;cursor:pointer;"><span>${esc(m)}</span></label></li>`).join('')}</ul></div>`;
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

async function answer(decision, choice, id = focused) {
  const g = (state.gates || []).find(x => x.id === id) || recordById(id);
  if (!g) return;
  const box = commentBox();
  const comment = box?.value.trim();
  // Sending back something the worker has moved past, without saying what is wrong, gives it
  // nothing to act on.
  if (g.wait === false && !comment) {
    note(`adj gate answer --id ${g.id} --decision changes`, true, '差し戻す理由をコメントに書いてください');
    return;
  }
  const line = `adj gate answer --id ${g.id} --decision ${decision}` + (choice ? ` --choice ${choice}` : '');
  try {
    const data = await api(`/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision, choice, comment }),
    });
    // The hub opened dispatch and issue gates, and their answers go to its inbox instead.
    const toHub = ['dispatch', 'issue'].includes(g.kind);
    note(line, false, toHub
      ? 'hub の受信箱に送信' + handedNote({ present: data.present, woken: data.woken })
      : `${g.worktree.split('/').pop()} の outbox に追記` +
        (data.woken ? ' → worker に通知しました' : data.present ? ' → worker は次回の outbox 確認時に読み込みます'
                                                               : ' → worker は停止中のため、回答は outbox で保持されます'));
    // A record stays, with this answer appended, so it stays in view to show that it went.
    if (g.wait === false && box) box.value = '';
    else if (focused === g.id) focused = null;
    await refresh();
  } catch (e) { note(line, true, e.message); }
}

/* The escape hatch from "見せて決める" to "話して決める". The gate stays open on purpose:
   the ball is still with the human until they come back and close it. */
function talk(id = focused) {
  const g = (state.gates || []).find(x => x.id === id);
  if (!g) return;
  // A gate the hub opened sits in the main checkout, where there is no worker: its tab is the
  // hub's.
  if (['dispatch', 'issue'].includes(g.kind)) focusHub(); else worktreeAct('focus', g.worktree);
  note('gate は開いたままです', false, 'タブで確認後、「解決済みとして閉じる」を押してください');
}

async function closeGate(id = focused) {
  const g = (state.gates || []).find(x => x.id === id);
  if (!g) return;
  const comment = commentBox()?.value.trim();
  const line = `adj gate close --id ${g.id}` + (comment ? ` --comment '${comment}'` : '');
  try {
    await api(`/api/gates/${encodeURIComponent(g.id)}`, {
      method: 'POST', body: JSON.stringify({ decision: 'close', comment: comment || 'タブで解決済み' }),
    });
    note(line, false, '解決済みとしてアーカイブしました（worker への outbox 配信なし）');
    if (focused === g.id) focused = null;
    await refresh();
  } catch (e) { note(line, true, e.message); }
}

