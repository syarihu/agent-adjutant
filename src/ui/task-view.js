// ── what the task panel's tabs draw ───────────────────────────────────

/* The tab a gate of each kind is read in. The other kinds are the hub's or a question, and
   show only in 経過. */
const TAB_OF_KIND = { plan:'overview', diff:'review', verify:'check' };
const KIND_OF_TAB = { overview:'plan', review:'diff', check:'verify' };

/* What a task's gates left in the archive, read when the task panel opens
   on it rather than on every poll: the archive only grows. Read again when a gate of the task has been answered since,
   which is when it can have changed. */
const histories = {};
const historyFailed = new Set();
/* A task of another board (the Sessions sidebar reads those) is kept under its board's path,
   since task ids are only unique within a board; the ones of this page keep the bare id. */
const historyKey = (task, base) => base === BASE ? task.id : `${base}|${task.id}`;
function historyOf(task, base = baseOf(task), data = state) {
  // The polled records leave their diffs out and say how big each is, so a record that was
  // written or rewritten since shows as a different key here and its diff is read again.
  const sizes = (task.records || []).map(r => `${r.id}:${r.diffSize ?? ''}`).join(',');
  const key = `${task.gateAnsweredAt || ''}|${openGate(task, data)?.id || ''}|${task.status}|${sizes}`;
  const hk = historyKey(task, base);
  let entry = histories[hk];
  // After a failure the entry waits out `retryAt`: every poll redraws, and each would ask again.
  if (entry && entry.key !== key && entry.retryAt > Date.now() && entry.failedKey === key) return entry;
  if (!entry || entry.key !== key) {
    entry = histories[hk] = { key, answered: entry?.answered || [], records: entry?.records || [], loaded: !!entry?.loaded };
    const mine = entry;
    boardApi(base, `/api/tasks/${encodeURIComponent(task.id)}/history`).then(data => {
      if (histories[hk] !== mine) return;
      mine.answered = data.answered || [];
      mine.records = data.records || [];
      mine.loaded = true;
      historyFailed.delete(hk);
      redrawHistoryOf(task.id);
    }).catch(e => {
      // Asked for again on the next redraw, keeping what was read before on screen meanwhile.
      if (histories[hk] === mine) {
        mine.key = null;
        mine.failedKey = key;
        mine.retryAt = Date.now() + 30000;
      }
      // Said once per run of failures, so the retries below do not push the log of what was
      // done off the footer.
      if (!historyFailed.has(hk)) note(`経過を取得できませんでした: ${e.message}`, true);
      historyFailed.add(hk);
      // Nothing else redraws a quiet board, so the view asks again itself — while no comment
      // is being typed, since a redraw would cut an IME composition short.
      setTimeout(() => redrawHistoryOf(task.id), 30000);
    });
  }
  return entry;
}

/* Redraws what shows a task's history: the task panel open on it. The panel
   keeps what is being typed into its instruction box across a redraw (renderHandForm). */
function redrawHistoryOf(id) {
  if ((view === 'board' || view === 'work') && selectedTaskId === id) renderTaskPanel();
}

/* A record from /api/state has no diff, only `diffSize`; the diff is the history's copy of the
   same record. Without it yet (still loading) the record is returned as it is, and
   `diffPending` says to show that rather than "no diff". */
function withDiff(record, task, base = baseOf(task), data = state) {
  if (!record || record.diff != null || !record.diffSize || !task) return record;
  const kept = historyOf(task, base, data).records.find(r => r.id === record.id);
  return kept?.diff != null ? { ...record, diff: kept.diff } : record;
}
const diffPending = g => !!g.diffSize && g.diff == null;
const DIFF_LOADING = `<div class="panel"><div class="empty-state">差分を読み込み中…</div></div>`;

/* Every gate of a task, oldest first: answered, kept as records, and waiting now. A live
   task's records come from /api/state, which is polled, so a send-back shows at once. */
function gatesOf(task, data = state, base = baseOf(task)) {
  const h = historyOf(task, base, data);
  const byId = new Map();
  // What the history holds belongs to the task's board; in a merged state it says so, as the
  // open gates do, or a gate picked from it could be taken for another board's.
  const own = g => g && task._slug ? { ...g, _slug: task._slug, _base: task._base } : g;
  const add = g => g && byId.set(g.id, g);
  h.answered.forEach(g => add(own(g)));
  add(own(task.approvedPlan));
  (task.records || h.records).forEach(r => add(own(task.records ? withDiff(r, task, base, data) : r)));
  (data.gates || []).filter(g => g.task === task.id && (!task._slug || g._slug === task._slug)).forEach(add);
  // Same-second ties go by the sequence at the end of the id, as `recordsOf` orders them, so
  // the latest of a kind is the one claimed last.
  return [...byId.values()].sort((a, b) =>
    (a.openedAt || '').localeCompare(b.openedAt || '') || claimSeq(a) - claimSeq(b));
}

/* The order a gate or record was claimed in within its second: `…-diff-2`, `…-record-2`, and
   none for the first. */
const claimSeq = g => +(/-(\d+)$/.exec(g.id)?.[1] || 1);

const isWaiting = g => (state.gates || []).some(x => gateRef(x) === gateRef(g));

/* The gate a tab shows: the one picked from 経過, else the one waiting, else the latest. `pick`
   is what was picked in the task panel. */
function gateForTab(task, tab, all, pick) {
  const picked = pick[tab] && all.find(g => g.id === pick[tab]);
  if (picked) return picked;
  const ofKind = all.filter(g => g.kind === KIND_OF_TAB[tab]);
  return ofKind.find(isWaiting) || ofKind[ofKind.length - 1] || null;
}

/* The files a unified diff touches, with the lines each gained and lost. */
function filesOf(diff) {
  const files = [];
  let cur = null, inHeader = false;
  for (const l of (diff || '').split('\n')) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(l);
    if (m) { cur = { path: m[2], add: 0, del: 0 }; files.push(cur); inHeader = true; continue; }
    if (l.startsWith('@@')) { inHeader = false; continue; }
    // A removed line that begins `--` is content, not a header: headers end at the first hunk.
    if (!cur || inHeader) continue;
    if (l.startsWith('+')) cur.add++;
    else if (l.startsWith('-')) cur.del++;
  }
  return files;
}

/* One line saying where a gate stands: waiting, recorded, or answered and how. */
function gateStatusHtml(g) {
  if (isWaiting(g)) return `<span class="state warn" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">hourglass_empty</span><span>${ago(g.openedAt)}から人の判定を待っている</span></span>`;
  if (g.wait === false) {
    const sent = (g.answers || []).length;
    return `<span class="state good" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-success);">check_circle</span><span>${ago(g.openedAt)}に記録 — worker は止まらずに進んだ${sent ? `（差し戻し ${sent}回）` : ''}</span></span>`;
  }
  if (g.decision) {
    const bad = ['changes', 'reject'].includes(g.decision);
    return `<span class="state ${bad ? 'bad' : 'good'}" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:${bad ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-success)'};">${bad ? 'replay' : 'check_circle'}</span>` +
      `<span title="${esc(when(g.answeredAt))}">${ago(g.answeredAt)}に人が${esc(DECISION[g.decision] || g.decision)}</span></span>` +
      (g.comment ? ` <span style="color:var(--md-sys-color-on-surface-variant)">— ${esc(g.comment)}</span>` : '');
  }
  return '';
}

/* The head of a tab showing one gate: its title, where it stands, and a way to see the others
   of its kind when there are several. */
function gateHeadHtml(g, all) {
  const same = all.filter(x => x.kind === g.kind);
  let h = `<div class="panel"><div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">` +
    `<b style="flex:1;min-width:0;overflow-wrap:anywhere">${esc(g.title)}</b>` +
    `<span class="mono2" style="color:var(--md-sys-color-outline)">${esc(when(g.openedAt))}</span></div>` +
    `<div style="margin-top:4px">${gateStatusHtml(g)}</div>`;
  if (stopWhy(g).length) {
    h += `<div class="state${stopBad(g) ? ' bad' : ''}" style="margin-top:6px;white-space:normal;display:flex;align-items:center;gap:6px;"><span class="material-symbols-outlined" style="font-size:16px;color:${stopBad(g) ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-warning)'};">${stopBad(g) ? 'error' : 'warning'}</span><span>止めた理由: ${esc(stopWhy(g).join(' / '))}</span></div>`;
  }
  if (same.length > 1) {
    h += `<div style="margin-top:8px;display:flex;gap:6px;flex-wrap:wrap;align-items:center">` +
      `<span style="color:var(--md-sys-color-outline);font-size:12px">このタスクの${esc(kindOf(g.kind)[0])} ${same.length}件:</span>` +
      same.map((x, i) => `<button type="button" class="chip${x.id === g.id ? ' good' : ''}" data-pick="${esc(x.id)}">` +
        `${i + 1}. ${esc(when(x.openedAt))}${x.wait === false ? ' 記録' : isWaiting(x) ? ' 待ち' : ''}</button>`).join('') + `</div>`;
  }
  return h + `</div>`;
}

/* The three frames, for a gate read in the task panel. */
function framesHtml(g) {
  return factsCardHtml(g) + focusCardHtml(g) + unsureCardHtml(g);
}

function factsCardHtml(g) {
  if (!g.facts?.length) return '';
  return `<div class="panel"${expandAttrs('facts', g)}>${expandBtnHtml('事実')}<h3><span class="material-symbols-outlined" style="font-size:18px;">info</span><span>事実</span></h3><ul>${g.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul></div>`;
}

function focusCardHtml(g) {
  if (!g.focus) return '';
  return `<div class="panel"${expandAttrs('focus', g)}>${expandBtnHtml('確認してほしい点')}<h3><span class="material-symbols-outlined" style="font-size:18px;">visibility</span><span>確認してほしい点</span></h3><div class="body frame-focus">${md(g.focus)}</div></div>`;
}

function unsureCardHtml(g) {
  if (!g.unsure) return '';
  return `<div class="panel"${expandAttrs('unsure', g)}>${expandBtnHtml('迷っていること')}<h3><span class="material-symbols-outlined" style="font-size:18px;">help</span><span>迷っていること</span></h3><div class="body">${md(g.unsure)}</div></div>`;
}

/* The worker's report on a gate. */
function reportCardHtml(g) {
  if (!g.body) return '';
  return `<div class="panel"${expandAttrs('report', g)}>${expandBtnHtml('報告')}<h3>報告</h3><div class="body">${md(g.body)}</div></div>`;
}

/* The files a diff touches, a row each. */
function filesTableHtml(g) {
  const files = filesOf(g.diff);
  if (!files.length) return '';
  return `<div class="panel"><h3>ファイル ${files.length}件</h3><div class="body"><table>` +
    `<tr><th>ファイル</th><th>追加</th><th>削除</th></tr>` +
    files.map(f => `<tr><td><code>${esc(f.path)}</code></td><td style="color:var(--good)">+${f.add}</td><td style="color:var(--critical)">−${f.del}</td></tr>`).join('') +
    `</table></div></div>`;
}

/* The diff itself, or the note that it is on its way. */
function diffCardHtml(g) {
  if (g.diff) return `<div class="panel"${expandAttrs('diff', g)}>${expandBtnHtml('差分')}<h3>差分</h3><div class="diff">${renderDiff(g.diff)}</div></div>`;
  return diffPending(g) ? DIFF_LOADING : '';
}

/* The part of a tab a person acts on, when the gate shown can still take an answer: a gate
   waiting now, or a live task's record. A finished task's records have nowhere to go back to. */
function actHtml(g) {
  if (isWaiting(g) || (g.wait === false && recordByRef(gateRef(g)))) return decideHtml(g);
  return '';
}

// The number at the end of an issue URL, ignoring a query, a fragment, or a trailing slash.
function issueNumberOf(url) {
  try { return new URL(url).pathname.split('/').filter(Boolean).pop() || ''; } catch { return ''; }
}
// Whether a URL is an issue the server can read (GitHub's shape); the server checks it again.
const isGithubIssue = u => /^https?:\/\/[^/?#]+\/[^/?#]+\/[^/?#]+\/issues\/\d+\/?(?:[?#]|$)/.test(u || '');
// The number of a pull request URL, also when it points at a tab of it (`…/pull/82/files`).
function prNumberOf(url) {
  try { return /\/pull\/(\d+)/.exec(new URL(url).pathname)?.[1] || ''; } catch { return ''; }
}

/* The Issue and the PR of a task, as the card and the panel show them. What is known about the
   PR comes from the record's `prStatus`, which the PR refresh writes: the page never asks
   GitHub, so a PR that has not been refreshed yet is shown as not yet checked. */
function prStateOf(task) {
  const stored = task.prStatus?.state;
  // A finished task is no longer refreshed, so a stored open or draft would never change.
  if (task.status === 'done') return task.pr ? 'merged' : null;
  if (task.status === 'cancelled') return stored === 'merged' || stored === 'closed' ? stored : null;
  return stored || null;
}
const PR_STATES = {
  open: ['オープン', 'pill-good'],
  draft: ['下書き', 'pill-neutral'],
  merged: ['マージ済み', 'pill-purple'],
  closed: ['クローズ', 'pill-neutral'],
  unknown: ['未確認', 'pill-neutral'],
};
/* Whose turn a PR is, as the server reads it from the record (`prTurn`). Where there is one,
   it says more than the state alone: a PR waiting on somebody else's review is not the
   person's, and a closed one needs a decision. */
const PR_TURN = {
  'other-reviewer': ['レビュー待ち（他の人）', 'pill-neutral'],
  checks: ['bot・CI 待ち', 'pill-neutral'],
  changes: ['修正の依頼あり', 'pill-warn'],
  merge: ['マージ待ち', 'pill-good'],
  'ci-failed': ['CI 失敗', 'pill-err'],
  closed: ['閉じられた', 'pill-warn'],
  merged: ['マージ済み', 'pill-purple'],
  draft: ['下書き', 'pill-neutral'],
};
const prStateInfo = task => PR_STATES[prStateOf(task) || 'unknown'] || PR_STATES.unknown;
const prPillClass = task => (PR_TURN[task.prTurn] || prStateInfo(task))[1];
/* The turn as a pill, for a card that shows nothing else about the PR's state. A draft is
   not one: it stays as it was, with no pill of its own. */
const prTurnPill = task => PR_TURN[task.prTurn] && task.prTurn !== 'draft'
  ? `<span class="m3-pill ${PR_TURN[task.prTurn][1]}">${esc(PR_TURN[task.prTurn][0])}</span>` : '';
/* Said when the automatic check is failing, so a state that may be out of date says so. */
const prStale = () => (state.prPoll?.error ? '（自動確認が止まっています）' : '');
/* 「レビュー待ち · CI 通過」: the state, the review for an open PR, and the checks if it has any. */
/* The checks of a PR in a word, or null when it has none. */
function ciNoteOf(task) {
  const ci = task.prStatus?.ci;
  if (!ci || ci.pass + ci.fail + ci.pending === 0) return null;
  return ci.fail > 0 ? `CI 失敗 ${ci.fail}` : ci.pending > 0 ? 'CI 実行中' : 'CI 通過';
}
function prNoteOf(task) {
  const turn = PR_TURN[task.prTurn];
  if (turn) {
    // The turn says whose ball it is; the checks stay beside it where they add something.
    if (['merged', 'closed', 'draft', 'checks'].includes(task.prTurn)) return turn[0];
    const ci = ciNoteOf(task);
    if (task.prTurn === 'ci-failed') return ci || turn[0];
    return ci ? `${turn[0]} · ${ci}` : turn[0];
  }
  const st = prStateOf(task);
  const [label] = prStateInfo(task);
  if (st === 'merged' || st === 'closed' || !st) return label;
  const parts = [];
  if (st === 'draft') parts.push(label);
  else parts.push({ approved: '承認済み', changes: '修正依頼' }[task.prStatus?.review] || 'レビュー待ち');
  const ci = ciNoteOf(task);
  if (ci) parts.push(ci);
  return parts.join(' · ');
}
/* The number of a PR record, which is a URL or, when written by hand, a bare "123" or "#123". */
const prRefNumber = pr => (httpUrl(pr) ? prNumberOf(pr) : /^#?(\d+)$/.exec(pr || '')?.[1] || '');
/* Issue and PR chips for a card's header; each opens on GitHub. Grouped in one element so they
   sit together on the left instead of being spread apart by the header's space-between. */
function ghChipsHtml(task) {
  const issueUrl = httpUrl(task.issueUrl);
  const prUrl = httpUrl(task.pr);
  const issueNumber = issueUrl ? issueNumberOf(issueUrl) : '';
  const issueLabel = issueUrl ? issueLabelOf(issueUrl) : '';
  const prNumber = prRefNumber(task.pr);
  const issue = issueNumber
    ? `<a href="${esc(issueUrl)}" target="_blank" rel="noopener noreferrer" class="card-issue-link" title="${esc(issueLabel)} を開く">
          <span class="material-symbols-outlined" style="font-size:12px;">tag</span>
          <span>${esc(issueLabel)}</span>
        </a>`
    : '';
  // A PR that is not a link is still a PR: shown, but not clickable.
  const prLabel = `<span class="material-symbols-outlined" style="font-size:12px;" aria-hidden="true">merge</span><span>${prNumber ? `PR #${esc(prNumber)}` : esc(task.pr)}</span>`;
  const prTitle = esc(`PR${prNumber ? ` #${prNumber}` : ''}・${prNoteOf(task)}${prStale()}`);
  const prClass = `gh-pr ${esc(prStateOf(task) || 'unknown')}`;
  const pr = prUrl
    ? `<a href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer" class="${prClass}" title="${prTitle}">${prLabel}</a>`
    : task.pr ? `<span class="${prClass}" title="${prTitle}">${prLabel}</span>` : '';
  return issue || pr ? `<span class="card-gh-chips">${issue}${pr}</span>` : '';
}
/* The label of an issue link: GitHub's number, a tracker key (Jira `/browse/ABC-12`, Linear
   `/team/issue/ABC-12/slug`), or just "Issue" where the URL names neither. */
function issueLabelOf(url) {
  const last = issueNumberOf(url);
  if (/^\d+$/.test(last)) return `Issue #${last}`;
  try {
    const key = new URL(url).pathname.split('/').find(seg => /^[A-Z][A-Z0-9]+-\d+$/i.test(seg));
    if (key) return `Issue ${key.toUpperCase()}`;
  } catch { /* not a URL: no key to read */ }
  return 'Issue';
}
/* The task's Issue and PR as link buttons at the start of the panel's top line; each opens in a
   new tab. Only the ones the task has: the Issue from its URL alone, as on the card, the PR only
   where it is a link. */
function ghHeadLinksHtml(task) {
  if (!task) return '';
  const link = (url, icon, label, title) =>
    `<a class="btn-m3-tonal sess-act tp-head-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer" title="${esc(title)}">`
    + `<span class="material-symbols-outlined" aria-hidden="true">${icon}</span><span>${esc(label)}</span></a>`;
  const issueUrl = httpUrl(task.issueUrl);
  const prUrl = httpUrl(task.pr);
  let h = '';
  if (issueUrl) {
    const label = issueLabelOf(issueUrl);
    h += link(issueUrl, 'tag', label, `${label} を開く`);
  }
  if (prUrl) {
    const n = prRefNumber(task.pr);
    const label = n ? `PR #${n}` : 'PR';
    h += link(prUrl, 'merge', label, `${label}・${prNoteOf(task)}${prStale()}`);
  }
  return h;
}
/* The Issue and the PR as rows for the top of the task panel's タスクサマリ and of a gate's 判断: number and title, and for the PR its state, checks and review. Empty for a task with
   neither a PR to show nor an Issue. */
function ghRowsHtml(task) {
  if (!task) return '';
  const out = '<span class="material-symbols-outlined gh-out" aria-hidden="true">open_in_new</span>';
  const issueUrl = httpUrl(task.issueUrl);
  const prUrl = httpUrl(task.pr);
  let h = '';
  if (issueUrl) {
    const n = issueNumberOf(issueUrl);
    h += `<a class="gh-row" href="${esc(issueUrl)}" target="_blank" rel="noopener noreferrer"><span class="material-symbols-outlined" aria-hidden="true">tag</span><span class="gh-kind">Issue</span>${n ? `<span class="gh-num">#${esc(n)}</span>` : ''}<span class="gh-title">${esc(task.issueSnapshot?.title ?? task.title)}</span>${out}</a>`;
  }
  if (task.pr) {
    const n = prRefNumber(task.pr);
    const cls = prPillClass(task);
    const body = `<span class="material-symbols-outlined" aria-hidden="true">merge</span><span class="gh-kind">PR</span>${n ? `<span class="gh-num">#${esc(n)}</span>` : ''}<span class="gh-title">${esc(task.prStatus?.title ?? (n ? task.title : task.pr))}</span><span class="m3-pill ${cls}" title="${esc(prNoteOf(task) + prStale())}">${esc(prNoteOf(task))}</span>`;
    h += prUrl
      ? `<a class="gh-row" href="${esc(prUrl)}" target="_blank" rel="noopener noreferrer">${body}${out}</a>`
      : `<div class="gh-row">${body}</div>`;
  } else {
    h += '<div class="gh-row gh-none"><span class="material-symbols-outlined" aria-hidden="true">merge</span><span class="gh-kind">PR</span><span class="gh-title">PR はまだありません</span></div>';
  }
  return `<div class="gh-block">${h}</div>`;
}

/* The entry of the board's `parents` for this task's parent: in the tab of every board, the one
   its own board listed. */
function parentGroupOf(task) {
  const key = task.parentIssue?.key;
  return (state.parents || []).find(g => g.key === key && (!task._slug || g._slug === task._slug));
}

/* The 親タスク row: the parent the board joined to the task, where that came from, how many of its
   children are merged, and the place in a stack. A task whose parent is only in its record shows the
   record's own text. */
function parentRowHtml(task) {
  const p = task.parentIssue;
  if (!p) return task.parent ? esc(task.parent) : '';
  const name = p.number ? `#${p.number}${p.title ? ' ' + p.title : ''}` : p.key;
  const link = httpUrl(p.url)
    ? `<a href="${esc(p.url)}" target="_blank" rel="noopener noreferrer" style="color:var(--accent)">${esc(name)}</a>`
    : esc(name);
  const dim = text => ` <span style="color:var(--ink-2)">${esc(text)}</span>`;
  let h = link + dim(p.source === 'tracker' ? '(GitHub)' : '(記録)');
  const group = parentGroupOf(task);
  if (group) {
    h += dim(`${group.merged} / ${group.total} マージ`);
    const kids = group.children || [];
    const me = p.hub;
    // The chain a child is in: what is cut from the same root, in the order the server listed them.
    const same = (c, id, hub) => c.id === id && c.hub === hub;
    const upOf = c => c.on && kids.find(d => same(d, c.on, c.onHub));
    const rootOf = c => { let at = c; for (let i = 0; i <= kids.length && upOf(at); i++) at = upOf(at); return at; };
    const mine = kids.find(c => same(c, task.id, me));
    const inStack = c => c.on || kids.some(d => same(c, d.on, d.onHub));
    const chain = mine ? kids.filter(c => inStack(c) && rootOf(c) === rootOf(mine)) : [];
    const at = chain.indexOf(mine);
    if (group.stacked && at >= 0) h += dim(`stack ${at + 1}/${chain.length}`);
  }
  if (p.source === 'tracker' && p.recordKey && p.recordKey !== p.key) h += dim('記録と違う');
  if (httpUrl(p.url)) {
    h += ` <button type="button" class="iconbtn" data-add-child="${esc(p.url)}" data-add-child-board="${esc(task._slug || '')}">子タスクを足す</button>`;
  }
  return h;
}

/* Where the problem and the goal came from: the plan's own words when the worker wrote them,
   otherwise the request as it was handed over. */
function planSourceOf(task, plan) {
  const issue = httpUrl(task.issueUrl);
  const source = issue
    ? `Issue <a href="${esc(issue)}" target="_blank" rel="noopener noreferrer">${esc(issue)}</a>`
    : task.issueUrl ? `Issue ${esc(task.issueUrl)}` : '依頼文';
  return { source, fromPlan: plan && `計画「${esc(plan.title)}」— worker が${source}を読んで書いたもの` };
}

function problemCardHtml(task, plan) {
  const { source, fromPlan } = planSourceOf(task, plan);
  const hasProblem = !!(plan?.problem || task.body);
  let h = `<div class="panel"${hasProblem ? expandAttrs('problem', plan?.problem ? plan : null) : ''}>${hasProblem ? expandBtnHtml('問題') : ''}<h3>問題</h3>`;
  if (plan?.problem) h += `<div class="body">${md(plan.problem)}</div><div class="source">出典: ${fromPlan}</div>`;
  else if (task.body) h += `<div class="body">${md(task.body)}</div><div class="source">出典: 渡したときの依頼文（計画に problem がまだ無い）</div>`;
  else h += `<div style="color:var(--muted)">まだ書かれていない${task.issueUrl ? ` — ${source}` : ''}</div>`;
  return h + `</div>`;
}

/* What the issue itself said when the task started, kept on the record. Rendered by md()
   like the request: it is text from a tracker, not markup. */
function issueBodyCardHtml(task) {
  const snap = task.issueSnapshot;
  if (!snap) return '';
  const snapLink = httpUrl(snap.url);
  return `<div class="panel"${snap.body ? expandAttrs('issue') : ''}>${snap.body ? expandBtnHtml('Issue の本文') : ''}<h3>Issue の本文</h3><div><b>${esc(snap.title)}</b></div>` +
    (snap.body ? `<div class="body">${md(snap.body)}</div>` : `<div style="color:var(--muted)">本文なし</div>`) +
    (snap.truncated ? `<div class="source">先頭のみ保存 — 続きは ${snapLink ? `<a href="${esc(snapLink)}" target="_blank" rel="noopener noreferrer">Issue</a>` : 'Issue'} で</div>` : '') +
    `<div class="source">出典: Issue #${esc(issueNumberOf(snap.url))} を ${esc(when(snap.fetchedAt))}（${esc(ago(snap.fetchedAt))}）に取得 ` +
    `<button type="button" class="iconbtn" data-fetch-issue="${esc(task.id)}">再取得</button></div></div>`;
}

function goalCardHtml(task, plan) {
  let h = `<div class="panel"${plan?.goal ? expandAttrs('goal', plan) : ''}>${plan?.goal ? expandBtnHtml('ゴール') : ''}<h3>ゴール</h3>`;
  if (plan?.goal) h += `<div class="body">${md(plan.goal)}</div><div class="source">出典: ${planSourceOf(task, plan).fromPlan}</div>`;
  else h += `<div style="color:var(--muted)">計画に goal がまだ無い</div>`;
  return h + `</div>`;
}

function instructionCardHtml(task) {
  if (!task.instruction) return '';
  return `<div class="panel" style="border-left: 3px solid var(--md-sys-color-primary, #6750A4);">` +
    `<h3>エージェントへの申し送り（指示）</h3>` +
    `<div class="body" style="white-space:pre-wrap;font-size:13.5px;line-height:1.6;">${esc(task.instruction)}</div>` +
    `<div class="source">キュー投入時の指示</div></div>`;
}

/* The plan itself: its title, who approved it, the other plans to pick from. `plan` is null
   when none has come out yet. */
function planHeadCardHtml(plan, plans, opts = {}) {
  let h = `<div class="panel"${plan ? expandAttrs('plan', plan) : ''}>${plan ? expandBtnHtml('計画') : ''}<h3>計画</h3>`;
  if (!plan) return h + `<div style="color:var(--muted)">計画はまだ出ていない</div></div>`;
  // Who approved it: a plan gate is answered only by a person, on the board or with
  // `adj gate answer`; the gate does not record which one.
  const approved = ['approve', 'choice'].includes(plan.decision);
  h += `<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><b>${esc(plan.title)}</b></div>` +
    `<div style="margin-top:4px">${isWaiting(plan) ? `<span class="state warn" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-warning);">hourglass_empty</span><span>${ago(plan.openedAt)}から承認待ち</span></span>`
      : approved ? `<span class="state good" style="display:inline-flex;align-items:center;gap:4px;"><span class="material-symbols-outlined" style="font-size:16px;color:var(--md-sys-color-success);">check_circle</span><span title="${esc(when(plan.answeredAt))}">${esc(when(plan.answeredAt))}（${ago(plan.answeredAt)}）に人が${esc(DECISION[plan.decision])}</span></span>` +
        (plan.comment ? ` <span style="color:var(--md-sys-color-on-surface-variant)">— ${esc(plan.comment)}</span>` : '')
      : gateStatusHtml(plan)}</div>`;
  // `opts.chips` false leaves out the row that picks another plan, for a page that cannot pick; `opts.focus` false
  // the focus, for a page that has it as a card of its own.
  if (plans.length > 1 && opts.chips !== false) {
    h += `<div style="margin-top:8px;display:flex;gap:6px;flex-wrap:wrap;align-items:center">` +
      `<span style="color:var(--muted);font-size:12px">計画 ${plans.length}件:</span>` +
      plans.map((x, i) => `<button type="button" class="chip${x.id === plan.id ? ' good' : ''}" data-pick="${esc(x.id)}">` +
        `${i + 1}. ${esc(when(x.openedAt))}${isWaiting(x) ? ' 待ち' : x.decision ? ` ${esc(DECISION[x.decision] || x.decision)}` : ''}</button>`).join('') + `</div>`;
  }
  if (plan.focus && opts.focus !== false) h += `<div class="body frame-focus" style="margin-top:10px">${md(plan.focus)}</div>`;
  return h + `</div>`;
}

/* What the plan gate carried beside its head, ending in the part a person acts on. */
function planGateCardsHtml(plan) {
  if (!plan) return '';
  let h = '';
  if (plan.facts?.length) h += `<div class="panel"${expandAttrs('facts', plan)}>${expandBtnHtml('事実')}<h3>事実</h3><ul>${plan.facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul></div>`;
  h += reportCardHtml(plan);
  h += choicesHtml(plan, isWaiting(plan));
  if (plan.unsure) h += `<div class="panel"${expandAttrs('unsure', plan)}>${expandBtnHtml('迷っていること')}<h3>迷っていること</h3><div class="body">${md(plan.unsure)}</div></div>`;
  if (plan.decided) h += `<div class="panel"${expandAttrs('decided', plan)}>${expandBtnHtml('決定事項')}<h3>決定事項</h3><div class="body">${md(plan.decided)}</div></div>`;
  return h + actHtml(plan);
}

/* The 詳細 list. `opts.panel` is the task panel's: its top already shows the Issue and the PR,
   and `opts.handForm` is set when its hand-over form shows the 申し送り, so neither is said
   twice. `opts.refs` keeps the Issue and PR rows anyway, and `opts.park` shows the 置いている
   row (the panel's by default), for the dialog that has neither at its top. */
function detailsKvHtml(task, opts = {}) {
  const rows = [
    ['タスクID', `<span class="mono2">${esc(task.id)}</span>`],
    ['完了条件', esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '—')],
    ['止める所', esc(STOP_AT[task.stopAt || 'plan'] || task.stopAt)],
    ['着手設定', task.autoStart ? '確認なしで着手' : '着手前に確認が必要'],
  ];
  const refRows = !opts.panel || opts.refs;
  if (task.instruction && !opts.handForm) rows.push(['申し送り', `<span style="white-space:pre-wrap">${esc(task.instruction)}</span>`]);
  const link = u => httpUrl(u) ? `<a href="${esc(u)}" target="_blank" rel="noopener noreferrer" style="color:var(--accent)">${esc(u)}</a>` : esc(u);
  const issueRef = task.issueUrl || task.issue;
  if (issueRef) {
    const fetchButton = !task.issueSnapshot && isGithubIssue(issueRef)
      ? ` <button type="button" class="iconbtn" data-fetch-issue="${esc(task.id)}">本文を取得</button>` : '';
    // The panel's top has the link; the button to read the body stays.
    if (refRows) rows.push(['Issue', link(issueRef) + fetchButton]);
    else if (fetchButton) rows.push(['Issue', fetchButton.trim()]);
  }
  if (task.pr && refRows) rows.push(['PR', link(task.pr)]);
  if (task.executor === 'jules') {
    rows.push(['実装', task.jules?.url ? `Jules ${esc(julesText(task.jules))} — ${link(task.jules.url)}`
      : task.julesSession ? `Jules ${task.jules ? esc(julesText(task.jules)) : ''}（session <span class="mono2">${esc(task.julesSession)}</span>）`
      : 'Jules（計画の承認後に渡す）']);
  }
  const parentHtml = parentRowHtml(task);
  if (parentHtml) rows.push(['親タスク', parentHtml]);
  if ((opts.park ?? opts.panel) && !['done', 'cancelled'].includes(task.status)) {
    const park = parkOf(task);
    const since = park && stampSecs(park.since) != null ? ` · ${esc(ago(park.since))}から` : '';
    rows.push(['置いている', park
      ? `${esc(parkText(park))}${since} <button type="button" class="iconbtn" ${parkAttrs(task, true)}>置くのをやめる</button>`
      : `<span style="color:var(--muted)">置いていない</span> <button type="button" class="iconbtn" ${parkAttrs(task, false)} title="誰かの返事やタイミングを待つので、「いまの仕事」の新着から外して置く">置く</button>`]);
  }
  if (task.branch) rows.push(['ブランチ', `<span class="mono2">${esc(task.branch)}</span>`]);
  if (task.base) rows.push(['分岐元', `<span class="mono2">${esc(task.base)}</span>`]);
  if (task.worktree) rows.push(['worktree', `<span class="mono2">${esc(task.worktree)}</span> <button type="button" class="iconbtn" title="${ideTitle()}" data-ide="${esc(task.worktree)}">IDE で開く</button>`]);
  rows.push(['作成', `${esc(when(task.createdAt))}（${ago(task.createdAt)}）`]);
  if (task.note) rows.push(['ノート', `<span style="white-space:pre-wrap">${esc(task.note)}</span>`]);
  return `<div class="panel"><h3>詳細</h3><dl class="kv">${rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('')}</dl></div>`;
}

/* `opts` is what `detailsKvHtml` takes: the task panel's `panel` and `handForm`. */
function overviewTab(task, all, pick, opts = {}) {
  const plans = all.filter(g => g.kind === 'plan');
  const plan = gateShownIn(task, 'overview', all, pick);
  return problemCardHtml(task, plan) + issueBodyCardHtml(task) + goalCardHtml(task, plan) +
    (opts.handForm ? '' : instructionCardHtml(task)) +
    planHeadCardHtml(plan, plans) + planGateCardsHtml(plan) + detailsKvHtml(task, opts);
}

function reviewTab(task, all, pick) {
  const g = gateForTab(task, 'review', all, pick);
  if (!g) return `<div class="empty-state">コードレビューはまだ無い。worker がセルフレビューを終えると、ここに出る。</div>`;
  // A gate waiting is judged on what the worker says about it, so the decision sits right under
  // that, above the rounds, the findings and the diff it can be checked against. A record's
  // send-back stays at the end, after what it would be sent back about.
  const waiting = isWaiting(g);
  let h = gateHeadHtml(g, all) + framesHtml(g) + (waiting ? actHtml(g) : '') + reviewPanels(g);
  h += reportCardHtml(g) + filesTableHtml(g);
  if (g.decided) h += `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<details class="decided"><summary>決定事項</summary><div class="body">${md(g.decided)}</div></details></div>`;
  h += diffCardHtml(g);
  return h + (waiting ? '' : actHtml(g));
}

function checkTab(task, all, pick) {
  const g = gateForTab(task, 'check', all, pick);
  if (!g) return `<div class="empty-state">動作確認はまだ無い。worker が verify を回すと、ここに出る。</div>`;
  let h = gateHeadHtml(g, all);
  if (isWaiting(g)) {
    h += `<div class="work">
      <button class="big" title="${ideTitle()}" data-ide="${esc(g.worktree)}">IDE で開く</button>
      <span class="mono2">${esc(g.worktree)}</span>
      <span style="color:var(--ink-2)">— 確認後、下のボタンで判定してください</span></div>`;
  }
  h += framesHtml(g) + checkPanels(g) + reportCardHtml(g);
  if (g.run) h += `<div class="panel"${expandAttrs('run', g)}>${expandBtnHtml('動かし方')}<h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>`;
  if (g.decided) h += `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<details class="decided"><summary>決定事項</summary><div class="body">${md(g.decided)}</div></details></div>`;
  return h + actHtml(g);
}

/* All of one gate, for the kinds that have no tab of their own (a report, a question, the hub's
   gates): picked from 経過 and shown above it, so nothing a gate carried is left unreadable. */
function gateDetailHtml(g, all) {
  let h = `<div class="panel" style="border-color:var(--accent)"><div style="display:flex;gap:8px;align-items:center">` +
    `<span class="tag"><span class="dot" style="background:${esc(kindOf(g.kind)[1])}"></span>${esc(kindOf(g.kind)[0])}</span>` +
    `<span style="flex:1"></span><button type="button" class="iconbtn" data-unpick>閉じる</button></div></div>`;
  h += gateHeadHtml(g, all) + framesHtml(g) + choicesHtml(g, isWaiting(g)) + reviewPanels(g) + checkPanels(g);
  if (g.body) h += `<div class="panel"${expandAttrs('report', g)}>${expandBtnHtml('報告')}<h3>報告</h3><div class="body">${md(g.body)}</div></div>`;
  if (g.run) h += `<div class="panel"${expandAttrs('run', g)}>${expandBtnHtml('動かし方')}<h3>動かし方</h3><div class="diff"><div>${esc(g.run).split('\n').join('</div><div>')}</div></div></div>`;
  if (g.decided) h += `<div class="panel"${expandAttrs('decided', g)}>${expandBtnHtml('決定事項')}<h3>決定事項</h3><div class="body">${md(g.decided)}</div></div>`;
  if (g.diff) h += `<div class="panel"${expandAttrs('diff', g)}>${expandBtnHtml('差分')}<h3>差分</h3><div class="diff">${renderDiff(g.diff)}</div></div>`;
  else if (diffPending(g)) h += DIFF_LOADING;
  return h + actHtml(g);
}

/* In time order: what waited on a person, what was only recorded, and what people did. */
function historyEventsOf(task, all, waiting = isWaiting) {
  const events = [];
  const push = (stamp, html, gateId) => events.push({ at: stampSecs(stamp) ?? 0, stamp, html, gateId });
  push(task.createdAt, `<div>タスクを作成</div><div class="who">${esc(DONE_WHEN[task.doneWhen] || task.doneWhen || '')} · ${esc(STOP_AT[task.stopAt || 'plan'] || '')}</div>`);
  for (const g of all) {
    const [label, colour] = kindOf(g.kind);
    const tag = `<span class="tag"><span class="dot" style="background:${esc(colour)}"></span>${esc(label)}</span>`;
    const title = `<button type="button" class="linkish" data-open="${esc(g.id)}">${esc(g.title)}</button>`;
    const why = stopWhy(g).length ? `<div class="who"${stopBad(g) ? ' style="color:var(--critical)"' : ''}>止めた理由: ${esc(stopWhy(g).join(' / '))}</div>` : '';
    const opened = g.wait === false ? 'worker が記録して、止まらずに進んだ'
      : waiting(g) ? 'worker が人を待っている' : 'worker が人を待った';
    let extra = '';
    if (g.kind === 'diff' || g.kind === 'verify') {
      const [text, tone] = recordSummary(g);
      extra = ` <span class="${tone === 'bad' ? 'state bad' : tone === 'good' ? 'state good' : ''}">${esc(text)}</span>`;
    }
    push(g.openedAt, `<div>${tag} ${title}${extra}</div><div class="who">${opened}</div>${why}`, g.id);
    if (g.answeredAt && g.decision) {
      push(g.answeredAt, `<div>人が${esc(DECISION[g.decision] || g.decision)} — ${esc(label)}「${esc(g.title)}」</div>` +
        (g.comment ? `<div class="who">${esc(g.comment)}</div>` : ''), g.id);
    }
    for (const a of g.answers || []) {
      push(a.answeredAt, `<div>人が差し戻した — ${esc(label)}の記録「${esc(g.title)}」</div>` +
        (a.comment ? `<div class="who">${esc(a.comment)}</div>` : ''), g.id);
    }
  }
  return events.sort((a, b) => a.at - b.at);
}

/* The timeline of 経過, drawn by the 経過 tab. It holds the gates and what people did, not the
   worker's phases: those are kept (`session.phases`) but read by 離れていた間に (my-work-away.js),
   so the phase it is in now closes this list rather than running through it. */
function timelineHtml(task, all, data = state, base = baseOf(task)) {
  // The waiting gates are those of the board the task is on, not of this page's.
  const events = historyEventsOf(task, all, data === state ? isWaiting : g => (data.gates || []).some(x => x.id === g.id));
  const worker = ['dispatched', 'pr'].includes(task.status) ? workerOf(task, data) : null;
  let h = '';
  h += `<ol class="timeline">` + events.map(e =>
    `<li><span class="at" title="${esc(ago(e.stamp))}">${esc(when(e.stamp))}</span><div class="what">${e.html}</div></li>`).join('');
  if (worker && worker.present && worker.phase) {
    const mins = phaseMinutes(worker);
    h += `<li><span class="at">いま</span><div class="what"><div>worker は${esc(PHASE_LABEL[worker.phase] || worker.phase)}` +
      `${mins != null ? `（${minutesLabel(mins)}前から）` : ''}</div></div></li>`;
  }
  h += `</ol>`;
  if (!histories[historyKey(task, base)]?.loaded) h += `<div class="source">回答済みのものを読み込んでいる…</div>`;
  return h;
}

function historyTab(task, all, pick) {
  const picked = pick.history && all.find(g => g.id === pick.history);
  let h = picked ? gateDetailHtml(picked, all) : '';
  return h + `<div class="panel"><h3>経過</h3>${timelineHtml(task, all)}</div>`;
}

/* The gate a tab has on screen, if it shows one. */
function gateShownIn(task, tab, all, pick) {
  if (tab === 'history') return (pick.history && all.find(g => g.id === pick.history)) || null;
  if (tab === 'overview') {
    // The plan being worked to is the one approved last; a plan waiting now is the one to judge.
    // /api/state names it only for a live task, so a finished one finds it the same way.
    const plans = all.filter(g => g.kind === 'plan');
    const approved = plans.filter(g => ['approve', 'choice'].includes(g.decision))
      .sort((a, b) => (a.answeredAt || '').localeCompare(b.answeredAt || '')).pop();
    return (pick.overview && all.find(g => g.id === pick.overview))
      || plans.find(isWaiting) || task.approvedPlan || approved || plans[plans.length - 1] || null;
  }
  return gateForTab(task, tab, all, pick);
}
