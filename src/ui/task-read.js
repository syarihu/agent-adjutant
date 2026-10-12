/* ── A task's read-through: the tabs over the middle, what the middle says, and the reader's head and tabs, as markup ─────────
   A task whose PR has a record has the same read-through a review request has (docs/architecture.md, "A task's PR"). Selecting
   the 「AI 下読み」 tab of the work view shows it in the middle and the reader in the panel. Pure, like pr-read.js, which it builds
   on: `GET /api/tasks/<id>/ai`'s document ({ pr, record, hub }), the read-through of `GET /api/others/read` and the clock go in,
   strings and plain values come out, with no page and no storage, so `src/ui/tests/task-read.test.js` runs them under
   `node --test`. task-read-view.js draws and keeps what the page holds. Loaded after pr-diff.js. */

const TASK_READ_NO_PR_WHY = 'PR ができたあとに使えます';
const TASK_READ_UNREADABLE_WHY = 'この PR は AI 下読みできません';

/* The 「AI 下読み」 tab: whether it can be chosen, why not, the state of the read that is written beside its label, and whether a
   dot is drawn (the read is of an earlier push). `doc` is the task's document, or null until it has come. */
function taskReadTabState(task, doc, nowSecs) {
  const state = (sub, extra = {}) => ({ disabled: false, why: '', sub, dot: false, ...extra });
  if (!task?.pr) return state(TASK_READ_NO_PR_WHY, { disabled: true, why: TASK_READ_NO_PR_WHY });
  // The task names a PR that is not one this can read (another host, or not a PR).
  if (doc && doc.pr == null) return state(TASK_READ_UNREADABLE_WHY, { disabled: true, why: TASK_READ_UNREADABLE_WHY });
  const rec = doc?.record;
  if (!rec) return state('');
  const ai = othersAiView(rec, [], nowSecs);
  switch (ai.kind) {
    case 'queued': case 'running': case 'hung': return state('下読み中…');
    case 'stale': return state('前の push 時点', { dot: true });
    case 'failed': return state('失敗しました');
    case 'done': {
      const c = rec.ai?.counts;
      return state([['要注意', c?.attention], ['確認', c?.check]].filter(([, n]) => Number.isFinite(n)).map(([w, n]) => `${w} ${n}`).join(' · '));
    }
    default: return state('');
  }
}

/* The two tabs over the middle. `on` is 'task' or 'ai'; `ai` is `taskReadTabState`'s. A tab that cannot be chosen is disabled with
   why as its title, unless it is the one that is on (it can always be left). */
function taskReadTabsHtml(ai, on) {
  const tab = (id, label, controls, st = {}) => {
    const off = st.disabled && on !== id;
    const dot = st.dot ? '<span class="tp-dot" role="img" aria-label="前回の下読みのあとに push されました" title="前回の下読みのあとに push されました"></span>' : '';
    return `<button type="button" role="tab" id="wk-mid-tab-${id}" class="wk-mid-tab${on === id ? ' active' : ''}" data-mid-tab="${id}" aria-selected="${on === id}" aria-controls="${controls}"${off ? ` disabled title="${esc(st.why)}"` : ''}>`
      + `<span class="wk-mid-label">${esc(label)}</span>${st.sub ? `<span class="wk-mid-sub">${esc(st.sub)}</span>` : ''}${dot}</button>`;
  };
  return tab('task', 'タスク', 'wk-term-pane') + tab('ai', 'AI 下読み', 'task-read', ai);
}

/* The head over the middle in the 「AI 下読み」 tab. `rec` is the record, null before there is one. */
function taskReadHeadHtml(task, rec) {
  const number = rec?.number ?? prRefNumber(task?.pr);
  const sub = rec ? `${rec.title} · ${rec.repo}` : task?.title || '';
  return `<div class="wk-term-title">${esc(`AI 下読み${number ? ` — PR #${number}` : ''}`)}</div><div class="wk-term-sub">${esc(sub)}</div>`;
}

/* The card that says what the reader is: its state when there is a read to put it beside, and where it can be talked to. */
function taskReaderCard(rec, read, records, nowSecs) {
  const ai = othersAiView(rec, records, nowSecs);
  const lines = [];
  if (read && ai.text) lines.push(`<div class="tp-line">${esc(ai.text)}</div>`);
  if (rec?.ai?.readerClosedAt) lines.push('<div class="tp-muted">下読みの worker は閉じました。下読みは、このまま読めます。</div>');
  else if (prReaderOpen(rec)) lines.push('<div class="tp-muted">下読みの worker は右のターミナルにいます。差分のことは、そこで聞けます。</div>');
  if (!lines.length) return null;
  const s = { key: 'top:reader', label: '下読みの worker', html: lines.join('') };
  return { key: s.key, label: s.label, badge: '', wide: false, html: prSecHtml(s), expand: null };
}

/* The groups of the middle: the dialog's (`prDialogGroups`, whose `opts` these are), without 前回のレビューから, which compares with
   a review of yours, and with the reader's card first in 要点. The task's document holds its own record only, not the queue, so a
   queued read says 順番待ち without a place: with its record alone it would always be the first. */
function taskReadGroups(rec, read, hubView, nowSecs, opts = {}) {
  const records = [];
  const groups = prDialogGroups(rec, read, records, hubView, nowSecs, { readerWhere: '右のターミナル', ...opts }).filter(g => g.key !== 'since');
  const reader = taskReaderCard(rec, read, records, nowSecs);
  const top = groups.find(g => g.key === 'top');
  if (top && reader) top.cards.unshift(reader);
  return groups;
}

/* What the middle says for a task whose PR has no record yet, and one button. */
function taskReadNoRecordHtml(pr, hubView, asking) {
  return `<div class="m3-filled-card pr-sec"><div class="tp-ph-head">まだ AI 下読みしていません</div>`
    + `<div class="tp-muted">${esc(`AI 下読みすると、読むだけの worker が ${pr || 'この PR'} を先に読みます。そのあと、右のターミナルでその worker に差分のことを聞けます。`)}</div>`
    + `${prAiButtonHtml({ action: 'AI 下読み' }, hubView, asking)}</div>`;
}

/* What the 「AI 下読み」 tab says while the code-review hub cannot be asked: that it is stopped, with the button that starts it when this
   page can (the list's hub row starts it the same way), or why a read cannot be opened. `rec` is the record, null before there is
   one; a PR that is done is not read again, so says nothing. `starting` is a start of this page's that is out. Nothing when a read
   can be asked for. */
function taskReadHubHtml(rec, hubView, starting = false) {
  if (!hubView || hubView.canRead) return '';
  if (rec && !OTHERS_LIVE.includes(rec.state)) return '';
  const line = text => `<div class="tp-line" role="note">${esc(text)}`;
  if (hubView.state !== 'stopped') return `${line(hubView.why)}</div>`;
  if (!hubView.startable) return `${line('コードレビュー hub が止まっています。このページから起動できるのは terminal.preset が tmux のときだけです。ターミナルで adj hub --review を実行すると起動します。')}</div>`;
  return `${line('コードレビュー hub が止まっています。AI 下読みを頼むには、hub を起動してください。')} <button type="button" class="btn-m3-tonal" data-task-read-hub-start${starting ? ' aria-disabled="true"' : ''}>`
    + `<span class="material-symbols-outlined" aria-hidden="true">smart_toy</span><span>${starting ? '起動中…' : 'hub を起動'}</span></button></div>`;
}

/* The band over the read-through when the PR has moved on since it was made, with the button to read it again. Nothing otherwise. */
function taskReadBandHtml(rec, read, hubView, asking) {
  if (!rec?.ai?.stale) return '';
  // The commit the read is of: the stored read-through's, and until it has come the record's own.
  const made = read || rec.ai;
  const again = OTHERS_LIVE.includes(rec.state)
    ? ` <button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '下読みし直す')}</button>` : '';
  return `<div class="tp-line" role="note">${esc(prStaleText(made, rec))}${again}</div>`;
}

/* The panel's head for the reader: the PR and its repository, what the reader may do, 「下読みを閉じる」 while it runs, and the
   panel's own buttons. `closing` is a close of this page's that is out. */
function taskReaderHeadHtml(rec, closing) {
  const url = httpUrl(rec?.url);
  const chip = url
    ? `<a class="btn-m3-tonal sess-act tp-head-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer" title="GitHub で PR #${esc(rec.number)} を開く"><span class="material-symbols-outlined" aria-hidden="true">merge</span><span>PR #${esc(rec.number)}</span></a>`
    : `<span class="tp-key">PR #${esc(rec?.number)}</span>`;
  return `<div class="tp-head-main">
      <div class="tp-badges">
        ${chip}
        <span class="origin-chip" title="${esc(rec?.repo)}"><span class="material-symbols-outlined" aria-hidden="true">folder</span><span>${esc(rec?.repo)}</span></span>
        <span class="m3-pill pill-neutral">読むだけ</span>
      </div>
      <h2 class="tp-title">下読みの worker</h2>
    </div>${panelBtnsHtml(prCloseReaderBtnHtml(rec, closing))}`;
}

/* The panel's tabs for the reader: its terminal, and 「worker への指示」 with the number of drafts it holds. The second needs a
   read-through, and says why while there is none. */
function taskReaderTabsHtml(pane, draftCount = 0, hasRead = false) {
  const tab = (id, tabId, label, extra, off, controls) =>
    `<button type="button" role="tab" id="tp-tab-read-${tabId}" class="m3-seg-tab${pane === id ? ' active' : ''}" data-read-pane="${id}" aria-selected="${pane === id}" aria-controls="${controls}"${off ? ` disabled title="${esc(off)}"` : ''}>${esc(label)}${extra}</button>`;
  const badge = draftCount > 0 ? `<span class="m3-tab-badge" style="background:var(--md-sys-color-surface-container-highest);color:var(--md-sys-color-on-surface);">${draftCount}</span>` : '';
  return tab('ai', 'term', 'ターミナル', '', '', 'tp-term') + tab('ai-drafts', 'drafts', 'worker への指示', badge, hasRead ? '' : PR_NO_READ_WHY, 'tp-detail');
}
