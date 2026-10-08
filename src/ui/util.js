/* Helpers that read nothing but their arguments, and the words for the vocabulary the server
   defines: gate kinds, decisions, worker phases. Loaded first, so every view can use them. */

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const stampSecs = stamp => {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(stamp || '');
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) / 1000 : null;
};
const updatedMs = t => { const s = stampSecs(t?.updatedAt); return s != null ? s * 1000 : Date.now(); };

const pad2 = n => String(n).padStart(2, '0');
const baseName = path => (path || '').split('/').filter(Boolean).pop() || '';

const minutesLabel = mins => mins < 1 ? '1分未満' : mins < 60 ? `${mins}分` : mins < 1440 ? `${Math.floor(mins / 60)}時間` : `${Math.floor(mins / 1440)}日`;
/* Minutes since something → 「たった今」「4分前」「2時間前」「3日前」. Floors, like minutesLabel. */
const agoLabel = mins => mins < 1 ? 'たった今' : `${minutesLabel(mins)}前`;
/* Whole minutes from `secs` to `nowSecs`, never below 0. The caller passes its own clock. */
const minutesSince = (secs, nowSecs) => Math.max(0, Math.floor((nowSecs - secs) / 60));

function sinceLabel(secs) {
  if (secs == null) return '';
  return agoLabel(minutesSince(secs, Date.now() / 1000));
}

/* `20260922T041233Z` → 「4分前」. The stamp is UTC and says so; the reader wants neither. */
function ago(stamp) {
  const secs = stampSecs(stamp);
  if (secs == null) return stamp || '';
  return agoLabel(minutesSince(secs, Date.now() / 1000));
}

/* What a task can be parked for, in the words the page uses; the ids are `task::PARK_REASONS`, the server's. A reason
   this page does not know (written by a newer binary) is shown as it was written. */
const PARK_REASON_LABEL = { pdm: 'PdM の確認待ち', design: 'デザイナーの確認待ち', review: 'エンジニアのレビュー待ち', 'merge-timing': 'マージのタイミング待ち', other: 'その他' };
const parkLabel = p => PARK_REASON_LABEL[p?.reason] || p?.reason || '';
/* The park of a task as the page reads it: none once the task is finished, whatever is left on the record. */
const parkOf = t => t?.parked?.reason && t.status !== 'done' && t.status !== 'cancelled' ? t.parked : null;
/* 「PdM の確認待ち（資料を待つ）」 */
const parkText = p => `${parkLabel(p)}${p?.text ? `（${p.text}）` : ''}`;

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

/* `20260922T041233Z` in the reader's own time: 9/22 13:12. */
function when(stamp) {
  const secs = stampSecs(stamp);
  if (secs == null) return stamp || '';
  const d = new Date(secs * 1000);
  return `${d.getMonth() + 1}/${d.getDate()} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/* A URL a link may point at: http or https only. The issue URL is typed into a form and
   stored as given, and `esc` keeps it from breaking the markup but not from being a
   `javascript:` link. */
const httpUrl = u => /^https?:\/\//i.test(u || '') ? u : null;

/* What `adj phase --set` writes, in the words a card shows. */
const PHASE_LABEL = { plan:'計画', implement:'実装', 'self-review':'セルフレビュー', verify:'動作確認',
                      pr:'PR', 'pr-bots':'bot待ち', review:'レビュー対応', report:'報告' };

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
const kindOf = k => KINDS[k] || [k, '#898781'];

/* What each gate decision is called on a button, wherever it is drawn. A kind overrides only
   where the decision means something different for it. */
const DECISION_LABEL = { approve:'承認する', changes:'差し戻す', reject:'見送る', ack:'了解', ask:'追加で聞く', answer:'答える' };
const DECISION_LABEL_OF_KIND = {
  dispatch: { approve:'着手する' },
  issue:    { approve:'着手する' },
  verify:   { approve:'確認した', changes:'直してほしい' },
};
const decisionLabel = (decision, kind) =>
  DECISION_LABEL_OF_KIND[kind]?.[decision] || DECISION_LABEL[decision] || decision;

const DECISION = { approve:'承認した', changes:'差し戻した', reject:'見送った', choice:'案を選んだ',
                   ack:'了解した', ask:'追加で聞いた', answer:'答えた', closed:'解決済みとして閉じた', terminal:'ターミナルで答えた' };
