/* ── A task's read-through: the heading button and the dialog's words, what the dialog says, as markup ──────────────────────────
   A task whose PR has a record has the same read-through a review request has (docs/architecture.md, "A task's PR"). A task with a PR
   has 「下読みを見る」 (or 「下読みする」) in the heading of its terminal, which opens the read-through as a dialog (task-read-view.js).
   Pure, like pr-read.js, which it builds on: `GET /api/tasks/<id>/ai`'s document ({ pr, record, hub }), the read-through of
   `GET /api/others/read` and the clock go in, strings and plain values come out, with no page and no storage, so
   `src/ui/tests/task-read.test.js` runs them under `node --test`. task-read-view.js draws and keeps what the page holds. Loaded after pr-diff.js. */

const TASK_READ_UNREADABLE_WHY = 'この PR は AI 下読みできません';

/* The heading button of a task's terminal: 'see' (「下読みを見る」: a read-through is there, or the document has not come yet), 'ask'
   (「下読みする」: the document has come and the PR has none, and none is under way) or '' (no button: the task has no PR, or it is
   merged or closed). `doc` is the task's document, or null until it has come; `nowSecs` tells a read under way from one that has hung;
   `prState` is the PR's state as the page shows it (`prStateOf`: a board's task carries no state of its own). A PR whose record says it
   is done is not offered a read either (the server refuses one): it keeps 下読みを見る only for a read-through it has. */
function taskReadHeadMode(task, doc, nowSecs = Date.now() / 1000, prState = null) {
  if (!task?.pr || prState === 'merged' || prState === 'closed') return '';
  if (!doc || !doc.pr) return 'see';
  const rec = doc.record;
  if (!rec) return 'ask';
  // A record that says the PR is done has nothing to read and nothing to ask for, until the page's own state catches up.
  if (!OTHERS_LIVE.includes(rec.state) && !prHasRead(rec)) return '';
  return prHasRead(rec) || ['queued', 'running'].includes(othersAiView(rec, [], nowSecs).kind) ? 'see' : 'ask';
}

/* The line over the dialog's title (the title is `ownReadTitleText`): the repository, the PR and whose it is. The repository is the
   record's, else the one the PR's ref (`ref`, `owner/repo#N`) or the task's PR link names, spelled as the work document has it (`nwo`)
   when that is the same repository. */
function ownReadSubText(rec, task, { ref = '', nwo = '' } = {}) {
  const m = /^(.+)#(\d+)$/.exec(ref || '') || /^https?:\/\/[^/]+\/([^/]+\/[^/]+)\/pull\/(\d+)/.exec(task?.pr || '');
  const named = m?.[1] || '';
  const repo = rec?.repo || (nwo && nwo.toLowerCase() === named.toLowerCase() ? nwo : named);
  return [repo, `PR #${rec?.number ?? (prRefNumber(task?.pr) || m?.[2] || '')}`, 'あなたのタスク'].filter(Boolean).join(' · ');
}
function ownReadTitleText(task) {
  return `AI 下読み — ${task?.title || ''}`;
}

/* The line under the dialog's title: when the reader finished and what it counted, or that it is at work, or that nothing was read. */
function ownReadLineText(rec, nowSecs) {
  if (!rec) return 'まだ下読みしていません';
  // A reader that has hung is not at work (the body offers the read again): the same rule as `prNoReadHtml`.
  if (['queued', 'running'].includes(othersAiView(rec, [rec], nowSecs).kind)) return '下読み中です';
  if (!prHasRead(rec)) return 'まだ下読みしていません';
  const c = rec.ai?.counts;
  const counts = [['要注意', c?.attention], ['確認', c?.check], ['気になる', c?.concerns]].filter(([, n]) => Number.isFinite(n)).map(([w, n]) => ` · ${w} ${n}`).join('');
  const at = stampSecs(rec.ai?.finishedAt);
  if (at == null) return `下読み済みです${counts}`;
  const ago = agoLabel(minutesSince(at, nowSecs));
  return `下読みの worker が${ago === 'たった今' ? ago : `${ago}に`}読み終えました${counts}`;
}

/* The line as the dialog shows it: nothing until the task's document has come, or when its PR cannot be read (the body says which),
   since a document that is not there is not a PR that was never read. */
function ownReadHeadLine(doc, nowSecs) {
  return doc?.pr ? ownReadLineText(doc.record || null, nowSecs) : '';
}

/* The buttons at the right of the dialog's head: 「読み直す」 while a read-through exists and the PR is live, and 「下読みを閉じる」 while a
   reader runs. `read` is the stored read-through (undefined while it is on its way). */
function ownReadActsHtml(rec, read, hubView, asking, closing) {
  if (!rec) return '';
  const again = prHasRead(rec, read) && OTHERS_LIVE.includes(rec.state)
    ? `<button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '読み直す')}</button>` : '';
  return again + prCloseReaderBtnHtml(rec, closing);
}

/* The card that says what the reader is: its state when there is a read to put it beside, and where it can be talked to. */
function taskReaderCard(rec, read, records, nowSecs, { terminal = true } = {}) {
  const ai = othersAiView(rec, records, nowSecs);
  const lines = [];
  if (read && ai.text) lines.push(`<div class="tp-line">${esc(ai.text)}</div>`);
  if (rec?.ai?.readerClosedAt) lines.push('<div class="tp-muted">下読みの worker は閉じました。下読みは、このまま読めます。</div>');
  else if (terminal && prReaderOpen(rec)) lines.push('<div class="tp-muted">下読みの worker は右のターミナルにいます。差分のことは、そこで聞けます。</div>');
  if (!lines.length) return null;
  const s = { key: 'top:reader', label: '下読みの worker', html: lines.join('') };
  return { key: s.key, label: s.label, badge: '', wide: false, html: prSecHtml(s), expand: null };
}

/* The groups of a task's read-through: the review dialog's (`prDialogGroups`, whose `opts` these are), and with the reader's card first in 要点. A task's
   have no 前回のレビューから, which compares with a review of yours (`opts.since` keeps it, for a review request), and its document
   holds its own record only, not the queue, so a queued read says 順番待ち without a place: with its record alone it would always be
   the first (`opts.records` is the queue, for a review request). `opts.terminal` false is the dialog's, which has no reader's terminal to point at
   or to type into. */
function taskReadGroups(rec, read, hubView, nowSecs, opts = {}) {
  const { records = [], since = false, terminal = true, ...rest } = opts;
  const groups = prDialogGroups(rec, read, records, hubView, nowSecs, { readerWhere: terminal ? '右のターミナル' : '', noAsk: !terminal, ...rest }).filter(g => since || g.key !== 'since');
  const reader = taskReaderCard(rec, read, records, nowSecs, { terminal });
  const top = groups.find(g => g.key === 'top');
  if (top && reader) top.cards.unshift(reader);
  // A read-through with nothing else to say in 要点 still has the reader's card there, after 前回のレビューから when that is shown.
  else if (reader) groups.splice(groups.findIndex(g => g.key === 'since') + 1, 0, { key: 'top', label: '要点', badge: '', cards: [reader] });
  return groups;
}

/* What the read-through says while the code-review hub cannot be asked: that it is stopped, with the button that starts it when this
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

/* The review request's panel head for the reader: what it is (the PR and its repository are in the middle's heading and the list), what it may do,
   「下読みを閉じる」 while it runs, and the panel's own buttons. `closing` is a close of this page's that is out. */
function taskReaderHeadHtml(rec, closing) {
  return `<div class="tp-head-main">
      <div class="tp-badges">
        <span class="material-symbols-outlined" aria-hidden="true">auto_stories</span>
        <h2 class="tp-title">読み手</h2>
        <span class="tp-muted">— 読むだけ · コードレビュー hub</span>
      </div>
    </div>${panelBtnsHtml(prCloseReaderBtnHtml(rec, closing))}`;
}
