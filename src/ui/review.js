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
  relay:    ['Jules に回す指摘','#0f8a9d'],
};
/* The kinds the hub opens: their answers go to its inbox, and its tab is the one to raise. */
const HUB_KINDS = ['dispatch', 'issue', 'relay'];
/* Whether the hub is the one waiting on this gate: one of its kinds, or a plan it opened for a
   task handed to Jules, whose worktree has no worker in it. */
const hubsGate = g => HUB_KINDS.includes(g.kind) || g.openedBy === 'hub';
const kindOf = k => KINDS[k] || [k, '#898781'];

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

/* `20260922T041233Z` → 「4分前」. The stamp is UTC and says so; the reader wants neither. */
function ago(stamp) {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp || '');
  if (!m) return stamp || '';
  const then = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  return agoLabel(Math.max(0, Math.floor((Date.now() - then) / 60000)));
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
const reviewCurrent = () => reviewDone.get(focused)?.gate || gateByRef(focused);

/* The waiting gates by board, in the sidebar's order, the longest-waiting first. A gate
   answered here is out even if a poll that was already on its way still lists it. */
function reviewGroups() {
  const open = (state.gates || []).filter(g => !reviewDone.has(gateRef(g)));
  const slugs = multiBoard ? orderedBoards(readBoards()).map(b => b.slug) : [''];
  for (const g of open) if (!slugs.includes(g._slug || '')) slugs.push(g._slug || '');
  return slugs.map(slug => {
    const items = open.filter(g => (g._slug || '') === slug)
      .sort((a, b) => (a.openedAt || '').localeCompare(b.openedAt || '') || a.id.localeCompare(b.id));
    const b = boards.find(x => x.slug === slug);
    return { slug, items, title: b ? boardName(b) : repoName() || '', repo: b?.hub ? repoNameOf(b) : '' };
  }).filter(grp => grp.items.length);
}

function reviewRowHtml(g, cur, done) {
  const ref = gateRef(g);
  const [label] = kindOf(g.kind);
  return `<button type="button" class="review-inbox-item item${done ? ' done' : ''}" data-rv-item="${esc(ref)}" aria-current="${ref === cur}">
    <div class="rv-row-top">
      <span class="m3-pill ${g.kind === 'plan' ? 'pill-blue' : g.kind === 'diff' ? 'pill-purple' : 'pill-warn'}">${esc(label)}</span>
      <span class="w rv-row-when">${done ? esc(DECISION[done.decision === 'close' ? 'closed' : done.decision] || done.decision) : ago(g.openedAt)}</span>
    </div>
    <div class="t">${esc(g.title)}</div>
    <div class="rv-row-wt">${esc(g.worktree ? g.worktree.split('/').pop() : '')}</div>
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
  const live = r => !reviewDone.has(r) && (state.gates || []).some(g => gateRef(g) === r);
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
  const [label] = kindOf(g.kind);
  const b = boards.find(x => x.slug === g._slug);
  const at = reviewOrder.indexOf(ref);
  const prevOk = at > 0;
  const nextOk = at < 0 ? reviewOrder.length > 0 : at < reviewOrder.length - 1;
  return `<div class="rv-head-main">
      ${b ? `<span class="tag">${esc(boardName(b))}</span>` : ''}
      <span class="m3-pill pill-warn">${esc(label)}</span>
      ${done ? '<span class="m3-pill pill-neutral">処理済み</span>' : ''}
      <span class="rv-head-title">${esc(g.title)}</span>
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
  if (hubsGate(g)) return mine.find(s => s.kind === 'hub' && s.id === reviewHubOf(g));
  return g.worktree ? mine.find(s => s.kind === 'worker' && s.worktree === g.worktree) : undefined;
}

/* Whether the ターミナル tab can open for this item, without listing the sessions: the board
   serves terminals and what the item waits on is running. */
function reviewTermUsable(g) {
  if (!state.boardTerminal?.available) return false;
  if (hubsGate(g)) {
    return g._slug ? !!boards.find(b => b.slug === g._slug)?.hubPresent
      : !!(state.hubs || []).find(h => h.id === reviewHubOf(g))?.state?.present;
  }
  return !!g.worktree && !!(state.workers || []).find(w => w.worktree === g.worktree)?.present;
}

/* Whether the sessions of the item's board are in: a board served alone always has them. */
const reviewSessionsRead = g => !scopeAll() || state.reviewSessionsOf === g._slug;

function reviewTabsHtml(g) {
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
      <button type="button" class="btn-m3-text close" style="margin-left:auto;color:var(--md-sys-color-outline)" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします"><span class="material-symbols-outlined" style="font-size:16px;">done_all</span><span>解決済みとして閉じる</span></button>
    </div>
  </div>`;
}

/* The 判断 tab: one column, from what waits to the buttons that answer it. */
function reviewJudgeHtml(g, task, done) {
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
    // A record is sent back from the same form a task's view has.
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
  const ref = g ? gateRef(g) : null;
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
  reviewOrder = groups.flatMap(grp => grp.items.map(gateRef));
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
  const ref = g ? gateRef(g) : null;
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
      <div style="font-size:15px;font-weight:700;color:var(--md-sys-color-on-surface);">${reviewDone.size ? 'すべて処理しました' : '対応待ちの判定はありません'}</div>
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
   can be drawn, #rv-judge is held until the box is left, as the task view does: replacing the
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
      : `${g.worktree.split('/').pop()} の outbox に追記` +
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

