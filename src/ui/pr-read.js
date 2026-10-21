/* ── A PR waiting on your review, in the middle and in 「拡大」: what the read-through says, as markup ─────────
   Pure, like my-work-others.js: a record of `GET /api/others`, the read-through of `GET /api/others/read` and the drafts
   of `GET /api/others/drafts` (docs/architecture.md, "The read-through" and "Draft comments") go in, strings and plain values
   come out, with no page and no storage, so `src/ui/tests/pr-read.test.js` runs them under `node --test`. The task's
   renderers cannot be used for a PR: they read the board's tasks and gates, and a PR has no board. Loaded after
   my-work-others.js, whose `othersAiView` and friends it calls; pr-panel.js and pr-dialog.js draw what it returns.

   Everything the reader wrote is data and goes through `esc`, and nothing of it is Markdown: it is shown as the text it is. */

const PR_LEVEL = { attention: '要注意', check: '確認', info: '参考' };
const PR_CATEGORY = { logic: 'ロジック', ui: 'UI', doc: 'ドキュメント', test: 'テスト', resource: 'リソース' };
const PR_REVIEW_STATE = { approved: '承認', 'changes-requested': '変更を依頼', commented: 'コメント', requested: '依頼中' };
const PR_SINCE_STATUS = { addressed: '対応済み', replied: '返信だけ', untouched: '手つかず' };
/* The placeholder of a file's card in 「拡大」 where there is no annotated diff hook (a fallback: the page sets one). */
const PR_DIFF_PLACEHOLDER = 'この PR の差分はまだここに表示できません。GitHub で開いて読んでください。';

const prSha7 = sha => String(sha || '').slice(0, 7);
const prSubText = rec => `${rec?.repo || ''} #${rec?.number ?? ''}`;
/* When the stored read-through has to be read again: its head or the time it finished changed. */
const prReadKey = rec => `${rec?.ai?.sha || ''}|${rec?.ai?.finishedAt || ''}`;
/* Whether there is a read-through to show, or one on its way from the server: `read` is undefined while it is being fetched. */
const prHasRead = (rec, read) => !!read || (read === undefined && !!rec?.ai?.sha);

/* The reader's window can be attached to when its record says it is in a tmux window (the middle mounts it, and says so when it cannot).
   A reader that was closed has no window, whatever the record still names. */
const prReaderOpen = rec => ['running', 'ready'].includes(rec?.ai?.status) && !rec.ai.readerClosedAt && rec.ai.terminal?.backend === 'tmux' && !!rec.ai.terminal.window;
/* Why 「聞く」 is off: it types into the reader's terminal. */
const PR_ASK_OFF_WHY = 'worker が動いていません';
/* The box the person types the overall comment in, in the send dialog (pr-send.js). A comment on a line is typed in a box of the diff
   (`data-prd-box`, pr-diff-view.js). */
const PR_BOX_SEL = '.pr-overall';
/* What the middle and the panel say while the PR's record is not there: the records are still coming, could not be read, or lack it. */
const prMissingText = (doc, error) => doc ? 'この PR は、いまの一覧にありません' : error ? `読み込めませんでした: ${error}` : '読み込み中…';

/* 「下読みを閉じる」, while a reader runs that has a process to stop; `closing` is a close of this page's that is out. */
const prCloseReaderBtnHtml = (rec, closing) => prReaderOpen(rec) && rec.ai.pid
  ? `<button type="button" class="btn-m3-text tp-jump" data-pr-close-reader${closing ? ' disabled' : ''} title="読み取り専用の worker を止めて、ウィンドウを閉じます">${closing ? '閉じています…' : '下読みを閉じる'}</button>` : '';
/* The CI of a record as one word: any failure is 失敗 and its count, else a run under way, else 通過; なし when it has none. */
function prCiWord(ci) {
  const n = k => Number(ci?.[k]) || 0;
  if (n('fail') > 0) return `失敗 ${n('fail')}`;
  if (n('pending') > 0) return '実行中';
  return n('pass') > 0 ? '通過' : 'なし';
}

/* The button that opens the send dialog (pr-send.js): 「レビューを送る」 for a review request (`to` 'pending'), 「worker に渡す」 for a task's
   own read-through ('worker'). `n` is how many comments it would send now, shown in a badge when above 0; `off` is why it cannot be
   used, blank when it can. */
function prSendBtnHtml(to, n, off = '') {
  const worker = to === 'worker';
  const label = worker ? 'worker に渡す' : 'レビューを送る';
  const badge = n > 0 ? `<span class="pr-send-count">${n}</span>` : '';
  return `<button type="button" class="btn-m3-primary tp-jump" data-pr-send="${worker ? 'worker' : 'pending'}" aria-haspopup="dialog" aria-label="${esc(n > 0 ? `${label}（${n} 件）` : label)}"${off ? ` disabled title="${esc(off)}"` : ''}>`
    + `<span class="material-symbols-outlined" aria-hidden="true">${worker ? 'send' : 'rate_review'}</span><span>${label}</span>${badge}</button>`;
}
const PR_SEND_OFF_WHY = 'この PR はマージされたか、閉じられています。pending review には積めません';

/* The heading of a review request's middle: the number and title, the repository, author, size and CI under it, and at the right
   「GitHub で開く」, 「読み直す」, 「レビューを送る」 and 「拡大」. 「読み直す」 is there while a read-through exists and the PR is live and not
   out of review (the card of one that is has its own buttons); `hubView` and `asking` are what `prAskAttrs` takes. 「レビューを送る」 is there
   while a read-through exists and the drafts file is loaded (`sendN`, the comments a push would send now, `prPendable`'s; null until then), and
   off while the PR is not live. The heading is drawn when its markup changes, so what it counts is passed in. */
function prReviewHeadHtml(rec, read, hubView, asking = false, sendN = null) {
  const author = rec.author ? `@${rec.author}` : '（削除されたアカウント）';
  const sub = [rec.repo, author, `+${Number(rec.additions) || 0} −${Number(rec.deletions) || 0}`, `${Number(rec.changedFiles) || 0} files`, `CI ${prCiWord(rec.ci)}`].join(' · ');
  const url = httpUrl(rec.url);
  const open = url ? `<a class="btn-m3-text tp-jump" href="${esc(url)}" target="_blank" rel="noopener noreferrer" title="GitHub で開く（新しいタブ）"><span class="material-symbols-outlined" aria-hidden="true">open_in_new</span>GitHub で開く</a>` : '';
  const again = prHasRead(rec, read) && OTHERS_LIVE.includes(rec.state) && !othersOutOfReview(rec)
    ? `<button type="button" class="btn-m3-text tp-jump" data-pr-reread${prAskAttrs(hubView, asking)}><span class="material-symbols-outlined" aria-hidden="true">refresh</span>${esc(asking ? '依頼中…' : '読み直す')}</button>` : '';
  const send = prHasRead(rec, read) && sendN != null ? prSendBtnHtml('pending', sendN, OTHERS_LIVE.includes(rec.state) ? '' : PR_SEND_OFF_WHY) : '';
  const expand = '<button type="button" class="tool-btn" data-pr-expand aria-haspopup="dialog" title="拡大" aria-label="PR 全体を拡大して読む"><span class="material-symbols-outlined" aria-hidden="true">open_in_full</span></button>';
  return `<div class="wk-head-main"><div class="wk-term-title">${esc(`#${rec.number} ${rec.title}`)}</div><div class="wk-term-sub">${esc(sub)}</div></div><span class="wk-head-acts">${open}${again}${send}${expand}</span>`;
}

/* ── the sections: `{ key, label, html }`, or null when there is nothing to say ── */

/* A button that asks for a read: disabled for good while none can be asked for (the hub's `why` is its title); while a request is
   out it only ignores the click, as the list's does. */
const prAskAttrs = (hubView, asking) => `${hubView?.canRead ? '' : ` disabled title="${esc(hubView?.why || 'AI 下読みはまだ始められません')}"`}${asking ? ' aria-disabled="true"' : ''}`;
const prLi = html => `<li>${html}</li>`;
const prKv = (label, html) => `<div class="tp-kv"><span>${esc(label)}</span><strong>${html}</strong></div>`;
const prLoc = (path, line) => `<code class="pr-loc">${esc(path)}${line ? `:${esc(line)}` : ''}</code>`;
const prFileStat = (rec, path) => (rec?.files || []).find(f => f.path === path) || null;
const prFileStatText = f => f ? `+${Number(f.additions) || 0} −${Number(f.deletions) || 0}` : '';

function prFacts(rec, nowSecs) {
  if (!rec) return null;
  const author = rec.author ? `@${esc(rec.author)}` : '（削除されたアカウント）';
  const size = `<span class="add">+${Number(rec.additions) || 0}</span> <span class="del">−${Number(rec.deletions) || 0}</span> · ${Number(rec.changedFiles) || 0} files · ${esc(othersSize(rec))}`;
  const requested = [othersRequestedLabel(rec, nowSecs), rec.rerequest ? '頼み直し' : ''].filter(Boolean).join(' · ');
  const reviewers = (rec.reviewers || []).map(r => `${esc(r.login)}${r.team ? '（チーム）' : ''} ${esc(PR_REVIEW_STATE[r.state] || r.state || '')}`.trim());
  const ci = rec.ci || {};
  const ciText = [ci.pass, ci.fail, ci.pending].some(n => Number(n) > 0)
    ? `成功 ${Number(ci.pass) || 0} / 失敗 ${Number(ci.fail) || 0} / 実行中 ${Number(ci.pending) || 0}` : 'なし';
  const rows = [
    prKv('作者', author),
    prKv('規模', size),
    requested ? prKv('依頼', esc(requested)) : '',
    prKv('レビュアー', reviewers.length ? reviewers.join('<br>') : 'なし'),
    prKv('CI', esc(ciText)),
    prKv('ブランチ', `<code>${esc(rec.base)}</code> ← <code>${esc(rec.head)}</code>${rec.draft ? ' <span class="m3-pill pill-neutral">ドラフト</span>' : ''}`),
  ].filter(Boolean).join('');
  return { key: 'top:facts', label: 'PR の事実', html: `<div class="tp-kvs">${rows}</div>` };
}

/* 「前回のレビューから」: for a PR pushed since your review, or one whose read-through compares with it. With `opts.diff` the place of
   each comment is a button that goes to its line of the diff. */
function prSince(rec, read, opts = {}) {
  const since = read?.sinceReview;
  if (rec?.state !== 'pushed' && !since) return null;
  const n = rec?.commitsSinceReview;
  const count = Number.isFinite(n) ? `前回のレビューから +${n} コミット` : '前回のレビューのあとに push されました';
  let html;
  if (since) {
    const comments = (since.comments || []).map((c, i) => prLi(
      `<span class="pr-chip pr-since-${esc(c.status)}">${esc(PR_SINCE_STATUS[c.status] || c.status)}</span> ${opts.diff && c.path ? `<button type="button" class="pr-loc-btn" data-pr-goto-since="${i}" title="差分で見る">${prLoc(c.path, c.line)}</button>` : prLoc(c.path, c.line)} <span>${esc(c.note)}</span>`));
    const changes = (since.newChanges || []).map(c => prLi(`<code class="pr-loc">${esc(c.path)}</code> <span>${esc(c.note)}</span>`));
    html = `<div class="tp-line">${esc(count)}</div>`
      + (comments.length ? `<ul class="pr-list">${comments.join('')}</ul>` : '<div class="tp-muted">前回の指摘は記録されていません</div>')
      + (changes.length ? `<div class="pr-sub">新しく増えた変更</div><ul class="pr-list">${changes.join('')}</ul>` : '');
  } else {
    html = `<div class="tp-line">${esc(count)}</div><div class="tp-muted">${read ? 'この下読みには前回のレビューとの比較がありません。下読みし直すと入ります。' : 'AI 下読みすると、前回の指摘への対応を確かめます。'}</div>`;
  }
  return { key: 'since:list', label: '前回のレビューから', badge: Number.isFinite(n) ? `+${n}` : '', html };
}

/* Whether `read` is a read-through of the older form (version 1, with 見るポイント and drafts of its own): one that is not a split
   proposal and has a version below 2. A missing version counts as old only if it has `points` or `drafts`. */
function prOldForm(read) {
  if (!read || read.mode === 'split') return false;
  if (read.version === undefined || read.version === null) return Array.isArray(read.points) || Array.isArray(read.drafts);
  return Number(read.version) < 2;
}

/* The band over a read-through of the older form: that it is, and, while the PR is live, the button that reads it again in the new form. */
function prOldBand(rec, read, hubView, asking = false) {
  if (!prOldForm(read)) return null;
  const made = read.madeAt ? `（${esc(when(read.madeAt))}）` : '';
  const again = OTHERS_LIVE.includes(rec?.state)
    ? ` <button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '下読みし直す')}</button>で新しい形式になります。` : '';
  return { key: 'top:old', label: '前の形式', html: `<div class="tp-line">前の形式の下読みです${made}。見るポイントはそのまま出します。${again}</div>` };
}

function prThree(read) {
  const t = read?.three;
  if (!t) return null;
  const line = (label, text) => `<div class="pr-three-row"><span class="pr-three-label">${esc(label)}</span><span>${esc(text)}</span></div>`;
  return { key: 'top:three', label: '3行でいうと', html: `${line('目的', t.purpose)}${line('やり方', t.approach)}${line('影響', t.impact)}<p class="pr-essence"><b>この差分の本質:</b> ${esc(t.essence)}</p>` };
}

/* 見るポイント, which only a read-through of the older form has; 「コメントにする」 opens a comment on the point's line of the diff. `opts.diff` is whether 「差分で見る」 can be offered (the annotated diff is there); `opts.readerOpen` false turns 「聞く」 off, `opts.noAsk` leaves it out (a place with no reader to type to). */
function prPoints(read, opts = {}) {
  const points = read?.points || [];
  if (!points.length) return null;
  const items = points.map((p, i) => {
    const lv = PR_LEVEL[p.level] ? p.level : 'info';
    return `<li class="pr-point pr-lv-${lv}" data-pr-point="${i}">
      <div class="pr-point-head"><span class="pr-level">${esc(PR_LEVEL[p.level] || p.level)}</span>${prLoc(p.path, p.line)}</div>
      <div class="pr-point-title">${esc(p.title)}</div>
      <div class="pr-point-why">${esc(p.why)}</div>
      <div class="pr-point-acts">${opts.diff ? `<button type="button" class="btn-m3-text pr-act" data-pr-goto-diff="${i}">差分で見る</button>` : ''}${opts.noAsk ? '' : `<button type="button" class="btn-m3-text pr-act" data-pr-ask="${i}"${opts.readerOpen === false ? ` disabled title="${esc(PR_ASK_OFF_WHY)}"` : ''}>聞く</button>`}<button type="button" class="btn-m3-text pr-act" data-pr-todraft="${i}">コメントにする</button></div>
    </li>`;
  });
  return { key: 'points:list', label: '見るポイント', badge: String(points.length), html: `<ul class="pr-points">${items.join('')}</ul>` };
}

/* 読む順, then the files deleted whole. With `opts.diff` a path is a button that goes to that file of the diff. */
function prOrder(read, rec, opts = {}) {
  const order = read?.order || [];
  const deleted = read?.deleted || [];
  if (!order.length && !deleted.length) return null;
  const loc = (path, attr, i) => opts.diff
    ? `<button type="button" class="pr-loc-btn" ${attr}="${i}" title="差分で見る"><code class="pr-loc">${esc(path)}</code></button>` : `<code class="pr-loc">${esc(path)}</code>`;
  const items = order.map((o, i) => {
    const stat = prFileStatText(prFileStat(rec, o.path));
    return `<li class="${o.skippable ? 'pr-skip' : ''}">${loc(o.path, 'data-pr-goto-file', i)} <span class="pr-cat">${esc(PR_CATEGORY[o.category] || o.category)}</span>${stat ? ` <span class="tp-muted">${esc(stat)}</span>` : ''}${o.skippable ? ' <span class="pr-chip pr-skippable">飛ばしてよい</span>' : ''}${o.note ? `<div class="pr-note">${esc(o.note)}</div>` : ''}</li>`;
  });
  const gone = deleted.map((d, i) => prLi(`${loc(d.path, 'data-pr-goto-deleted', i)} 丸ごと削除 — ${d.reason ? `理由: ${esc(d.reason)}` : `作者への質問: ${esc(d.question)}`}`));
  return { key: 'order:list', label: '読む順', badge: String(order.length || ''),
    html: (items.length ? `<ol class="pr-order">${items.join('')}</ol>` : '') + (gone.length ? `<ul class="pr-list pr-deleted">${gone.join('')}</ul>` : '') };
}

function prDecisions(read) {
  const ds = read?.decisions || [];
  if (!ds.length) return null;
  const items = ds.map(d => prLi(`<div class="pr-dec">${esc(d.decision)}</div><div><span class="pr-three-label">得るもの</span> ${esc(d.gains)}</div><div><span class="pr-three-label">払うもの</span> ${esc(d.costs)}</div>`));
  return { key: 'more:decisions', label: '設計判断と対価', html: `<ul class="pr-list">${items.join('')}</ul>` };
}

function prTests(read) {
  const covered = read?.tests?.covered || [];
  const not = read?.tests?.notCovered || [];
  if (!covered.length && !not.length) return null;
  return { key: 'more:tests', label: 'テストが押さえている範囲',
    html: (covered.length ? `<ul class="pr-list">${covered.map(x => prLi(esc(x))).join('')}</ul>` : '')
      + (not.length ? `<div class="pr-sub">押さえていない:</div><ul class="pr-list">${not.map(x => prLi(esc(x))).join('')}</ul>` : '') };
}

function prUnwritten(read) {
  const xs = read?.unwritten || [];
  if (!xs.length) return null;
  return { key: 'more:unwritten', label: '本文に書かれていないこと', html: `<ul class="pr-list">${xs.map(x => prLi(esc(x))).join('')}</ul>` };
}

/* What is said of a read-through whose PR has moved on since: the commit its diff is of, and the head now. */
const prStaleText = (read, rec) => `この差分は ${prSha7(read.sha)} で下読みしたときのものです（PR の head は ${prSha7(rec.headSha)}）`;

/* The foot of the read-through: the commit it was made at, a warning and 「下読みし直す」 when the head has moved, and what a
   read still under way says (`ai` is `othersAiView`'s). */
function prFooter(rec, read, ai, hubView, asking = false) {
  if (!read) return null;
  const stale = !!rec?.ai?.stale;
  const made = read.madeAt ? ` · ${esc(when(read.madeAt))}` : '';
  const under = ai && ['queued', 'running', 'hung', 'failed'].includes(ai.kind) ? `<div class="tp-muted">${esc(ai.text)}</div>` : '';
  // The card of a PR out of review has the buttons for this already.
  const oorNow = !!othersOutOfReview(rec) && OTHERS_LIVE.includes(rec?.state);
  // A proposal is no read of a PR that is not out of review now, however current: the usual button is offered (`ai.action`).
  const readButton = `<button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : ai?.action)}</button>`;
  const again = oorNow ? '' : !stale ? (read.mode === 'split' && ai?.action ? ` ${readButton}` : '') : ` <span class="m3-pill pill-warn" title="PR の head が、下読みしたときから動いています">head は ${esc(prSha7(rec.headSha))}</span> <button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '下読みし直す')}</button>`;
  return { key: 'footer', label: '下読みの時点',
    html: `<div class="tp-muted">下読みした時点: <code>${esc(prSha7(read.sha))}</code>${made}</div>${again ? `<div class="pr-foot-acts">${again}</div>` : ''}${under}` };
}

/* Before a read-through: the GitHub facts are all there is, and one button. What a read in progress or failed says is
   `othersAiView`'s; `asking` is a request of this page that is not answered yet. A view of a split proposal (`mode`, set only while the PR is out of review) asks for
   another proposal. Not drawn by the page for a PR that is out of review before it has a read: `prOorActions` has its buttons. */
function prAiButtonHtml(ai, hubView, asking) {
  if (!ai?.action) return '';
  if (ai.mode === 'split') return prSplitButtonHtml(ai.action, hubView, asking);
  return `<button type="button" class="btn-m3-tonal pr-ai" data-pr-ai${prAskAttrs(hubView, asking)}><span class="material-symbols-outlined" aria-hidden="true">menu_book</span><span>${esc(asking ? '依頼中…' : ai.action)}</span></button>`;
}
function prSplitButtonHtml(label, hubView, asking) {
  return `<button type="button" class="btn-m3-tonal pr-ai" data-pr-split${prAskAttrs(hubView, asking)}><span class="material-symbols-outlined" aria-hidden="true">call_split</span><span>${esc(asking ? '依頼中…' : label)}</span></button>`;
}
/* What the middle says before a read-through exists, in place of the groups: a read that is queued or running, else that none was made
   (with why the last one failed, and the button that asks for one). `ai` is `othersAiView`'s. */
function prNoReadHtml(rec, ai, hubView, asking) {
  const icon = name => `<span class="material-symbols-outlined" aria-hidden="true">${name}</span>`;
  if (['queued', 'running'].includes(ai?.kind)) {
    // The queue place is the tail of the view's text ("…の順番待ち（1 番目）").
    const place = ai.kind === 'queued' ? /順番待ち.*$/.exec(ai.text || '')?.[0] : '';
    return `<div class="wk-mid-empty-in pr-noread" role="status">${icon('progress_activity')}<strong>下読み中です</strong>`
      + `<span>読み終わると、ここに要点と差分の解説が出ます。</span>${place ? `<span>${esc(place)}</span>` : ''}</div>`;
  }
  const failed = ['failed', 'hung'].includes(ai?.kind) && ai.text ? `<span>${esc(ai.text)}</span>` : '';
  return `<div class="wk-mid-empty-in pr-noread">${icon('auto_stories')}<strong>まだ下読みしていません</strong>`
    + `<span>コードレビュー hub の読み手が、要点をまとめ、差分の行に解説を書きます。</span>${failed}${prAiButtonHtml(ai, hubView, asking)}</div>`;
}
function prBeforeRead(rec, records, hubView, nowSecs, asking = false, readerWhere = '') {
  const ai = othersAiView(rec, records, nowSecs);
  // `readerWhere`: where the reader can be talked to, when that is not the middle (a task's is its sidebar).
  const text = ai.kind === 'none' || !ai.text
    ? `下読みするまでは GitHub の情報だけです。AI 下読みすると、要点・差分の行の解説と気になるところ・設計判断・テスト範囲ができます。${readerWhere ? `読んだあとは、${readerWhere}でその worker に差分のことを聞けます。` : ''}` : ai.text;
  return { key: 'top:ai', label: 'AI 下読み', html: `<div class="tp-line">${esc(text)}</div>${prAiButtonHtml(ai, hubView, asking)}` };
}

/* ── a PR whose change is too large to read ── */

const PR_OOR_TEXT = `1 ファイルの変更が ${OTHERS_OUT_OF_REVIEW_LINES.toLocaleString('en-US')} 行を超えているため、行ごとのレビューには向きません（丸ごと削除したファイル、lock・スナップショット・生成ファイルは数えません）。`;

/* The files that are over the limit, one line each. */
function prOorFilesHtml(oor) {
  const items = oor.files.map(f => prLi(`<code class="pr-loc">${esc(f.path)}</code> +${f.additions} −${f.deletions}（計 ${f.additions + f.deletions} 行）`));
  return `<ul class="pr-list">${items.join('')}</ul>`;
}

/* The buttons of a PR out of review, as labels (null for one that is not offered): 「分割を提案する」 unless a split proposal is
   stored (made again when its head has moved), and 「それでも AI 下読み」 unless a read-through of the whole PR is stored and its
   head has not moved. None while a read is under way (a reader that has hung is not under way: asking again takes it over), and
   none on a PR that waits for nothing of the person. `read` is the stored read-through (undefined while it is being fetched, which
   offers nothing). `ai` is `othersAiView`'s. */
function prOorActions(rec, read, ai, readError = false) {
  const none = { split: null, full: null };
  if (!OTHERS_LIVE.includes(rec?.state) || ['queued', 'running'].includes(ai?.kind)) return none;
  // A read-through that could not be fetched is no reason to offer nothing: the request does not depend on it.
  if (read === undefined && rec?.ai?.sha && !readError) return none;
  const stale = !!rec?.ai?.stale;
  const isSplit = read?.mode === 'split';
  return {
    // A proposal that failed or whose reader hung is asked for again even if the stored one is current.
    split: isSplit ? (stale || ['failed', 'hung'].includes(ai?.kind) ? '分割を提案し直す' : null) : '分割を提案する',
    full: read && !isSplit && !stale ? null : 'それでも AI 下読み',
  };
}

/* The two buttons, and what 「それでも AI 下読み」 costs. */
function prOorButtonsHtml(actions, hubView, asking) {
  const split = actions.split ? prSplitButtonHtml(actions.split, hubView, asking) : '';
  const full = actions.full
    ? `<button type="button" class="btn-m3-tonal pr-ai" data-pr-ai${prAskAttrs(hubView, asking)}><span class="material-symbols-outlined" aria-hidden="true">menu_book</span><span>${esc(asking ? '依頼中…' : actions.full)}</span></button>` : '';
  const note = actions.full ? '<div class="tp-muted">差分を全部読むので時間がかかります。</div>' : '';
  return `${split || full ? `<div class="pr-foot-acts">${split}${full}</div>` : ''}${note}`;
}

/* 「レビュー対象外」: the files over the limit and the two ways on, in the 下読み tab and in 「拡大」. Null for a PR that is not out
   of review, and for one that waits for nothing of the person. `ai` is `othersAiView`'s. */
function prOutOfReview(rec, read, ai, hubView, asking = false, readError = false) {
  const oor = othersOutOfReview(rec);
  if (!oor || !OTHERS_LIVE.includes(rec?.state)) return null;
  const under = ['queued', 'running', 'hung', 'failed'].includes(ai?.kind) && ai.text ? `<div class="tp-line">${esc(ai.text)}</div>` : '';
  return { key: 'top:oor', label: 'レビュー対象外',
    html: `<div class="tp-line">${esc(PR_OOR_TEXT)}</div>${prOorFilesHtml(oor)}${under}${prOorButtonsHtml(prOorActions(rec, read, ai, readError), hubView, asking)}` };
}

/* The split proposal of a read-through made as one: its overall comment, as text. It is also the overall comment of the send dialog,
   where it is edited (`to` 'pending': 「レビューを送る」; 'worker': 「worker に渡す」, a task's own). */
function prSplitProposal(read, to = 'pending') {
  if (read?.mode !== 'split') return null;
  const where = to === 'worker' ? '「worker に渡す」で編集・コピー・worker に渡せます。' : '「レビューを送る」で編集・コピー・pending review に積めます。';
  return { key: 'top:split', label: '分割案',
    html: `<div class="pr-split">${esc(read.overall?.body || '')}</div><div class="tp-muted">${where}</div>` };
}

/* The sections of the 下読み tab. `opts`: `diff` (the annotated diff is there), `asking`, `readError`, `readerWhere` (prBeforeRead's), `noAsk` (no 「聞く」). */
function prSections(rec, read, records, hubView, nowSecs, opts = {}) {
  const ai = othersAiView(rec, records, nowSecs);
  const hasRead = !!read;
  return [
    prFacts(rec, nowSecs),
    prOutOfReview(rec, read, ai, hubView, opts.asking, !!opts.readError),
    prSince(rec, read, { diff: opts.diff }),
    !hasRead && read !== undefined && !(othersOutOfReview(rec) && OTHERS_LIVE.includes(rec?.state)) ? prBeforeRead(rec, records, hubView, nowSecs, opts.asking, opts.readerWhere) : null,
    prOldBand(rec, read, hubView, opts.asking),
    prThree(read),
    // A task's own read-through (`noAsk`) hands its comments to the worker; a review request's go into a pending review.
    prSplitProposal(read, opts.noAsk ? 'worker' : 'pending'),
    // 見るポイント and 読む順 are the older form's; a newer read-through has its notes on the diff's lines, in the diff's file order.
    prOldForm(read) ? prPoints(read, { diff: opts.diff, readerOpen: prReaderOpen(rec), noAsk: opts.noAsk }) : null,
    prOldForm(read) ? prOrder(read, rec, { diff: opts.diff }) : null,
    prDecisions(read),
    prTests(read),
    prUnwritten(read),
    prFooter(rec, read, ai, hubView, opts.asking),
  ].filter(Boolean);
}

const prSecHtml = s => `<div class="m3-filled-card pr-sec" data-pr-sec="${esc(s.key)}">${secTitle(esc(s.label))}${s.html}</div>`;

/* The 下読み tab. A read-through that is `undefined` is still being fetched; an error keeps the last one (the caller passes
   it again) and says so. */
function prDetailHtml(rec, read, records, hubView, nowSecs, opts = {}) {
  const notes = [];
  if (read === undefined && !opts.readError) notes.push('<div class="tp-muted" role="status">下読みを読み込み中…</div>');
  if (opts.readError) notes.push(`<div class="wk-notice" role="alert">${esc(read ? '最新の下読みを読めませんでした（表示は前回のものです）' : `下読みを読めませんでした: ${opts.readError}`)}</div>`);
  const secs = prSections(rec, read, records, hubView, nowSecs, opts).map(prSecHtml);
  // The notes sit under the facts, which are always the first.
  return [secs[0], ...notes, ...secs.slice(1)].filter(Boolean).join('');
}

/* ── the reader's place in the panel, while there is no live terminal in it ── */

/* What the panel says over the reader's terminal while there is none (or over one that has ended): the read-through is in the middle. */
const PR_RECONNECT_BTN = '<button type="button" class="btn-m3-tonal" data-pr-reconnect><span class="material-symbols-outlined" aria-hidden="true">sync</span><span>再接続</span></button>';
/* Why an attach failed, as its own line, unless it only says again what the line over it says. */
const prFailedWhyHtml = why => why && why !== '接続できませんでした' ? `<div class="tp-muted">${esc(why)}</div>` : '';
function prMiddleHtml(rec, records, hubView, nowSecs, opts = {}) {
  const ai = othersAiView(rec, records, nowSecs);
  const oor = othersOutOfReview(rec);
  // A PR that is out of review offers its two ways on until a reader is at work on it, in either mode. `opts.read` is the stored
  // read-through, which says whether a proposal is there already.
  // Also for a reader that has hung, and for a read-through that is stale or whose window cannot be opened.
  const offers = ['none', 'failed', 'stale', 'hung'].includes(ai.kind) || opts.ended != null || !!opts.failed || (ai.kind === 'done' && !prReaderOpen(rec));
  if (oor && OTHERS_LIVE.includes(rec?.state) && offers) {
    const why = ai.kind !== 'none' && ai.text ? `<div class="tp-line">${esc(ai.text)}</div>` : '';
    const gone = opts.ended != null ? '<div class="tp-muted">worker は終わっています。</div>'
      : opts.failed ? `<div class="tp-muted">worker に接続できませんでした</div>${prFailedWhyHtml(opts.failed)}` : '';
    const closed = rec?.ai?.readerClosedAt ? '<div class="tp-line">AI 下読みの worker を閉じました</div>' : '';
    // Only a reader that is still there can be tried again: for another there is nothing to mount.
    const reconnect = gone && prReaderOpen(rec) ? PR_RECONNECT_BTN : '';
    return `<div class="tp-ph-head">レビュー対象外</div>${prOorFilesHtml(oor)}${why}${closed}${gone}${reconnect}`
      + '<div class="tp-muted">worker はファイルの構成と hunk の見出しだけを読んで、この PR の分割案を下書きにします。</div>'
      + prOorButtonsHtml(prOorActions(rec, opts.read, ai, !!opts.readError), hubView, opts.asking);
  }
  // Closed on purpose: nothing to reconnect to, and the read-through is all there is.
  if (rec?.ai?.readerClosedAt) {
    const again = OTHERS_LIVE.includes(rec.state)
      ? `<button type="button" class="btn-m3-tonal pr-ai" data-pr-ai${prAskAttrs(hubView, opts.asking)}><span class="material-symbols-outlined" aria-hidden="true">menu_book</span><span>${esc(opts.asking ? '依頼中…' : '下読みし直す')}</span></button>` : '';
    return `<div class="tp-ph-head">AI 下読みの worker を閉じました</div><div class="tp-muted">${esc('下読みは真ん中に残っています。')}</div>` + again;
  }
  // `opts.ended` is a worker that really has ended; `opts.failed` is why the attach to a running one failed, in words.
  if (opts.ended != null || opts.failed) {
    const head = opts.ended != null ? 'AI 下読みの worker は終わっています' : 'AI 下読みの worker に接続できませんでした';
    return `<div class="tp-ph-head">${esc(head)}</div>${prFailedWhyHtml(opts.failed)}<div class="tp-muted">${esc('下読みは真ん中に残っています。')}</div>`
      + (prReaderOpen(rec) ? PR_RECONNECT_BTN : '')
      + prAiButtonHtml(ai, hubView, opts.asking);
  }
  if (ai.kind === 'none') {
    return '<div class="tp-ph-head">まだ AI 下読みしていません</div>'
      + '<div class="tp-muted">AI 下読みすると、読むだけの worker がこの PR を先に読みます。そのあと、ここでその worker に差分のことを聞けます。</div>'
      + prAiButtonHtml(ai, hubView, opts.asking);
  }
  const mountable = prReaderOpen(rec);
  const note = mountable ? '接続しています…'
    : ['done', 'stale'].includes(ai.kind) ? `${ai.mode === 'split' ? '分割案' : '下読み'}は真ん中にあります。worker の画面は開けません。`
    : ai.kind === 'running' || ai.kind === 'hung' ? 'worker の画面は開けません（tmux のウィンドウが記録されていません）。' : '';
  return `<div class="tp-ph-head">${esc(ai.text)}</div>${note ? `<div class="tp-muted">${esc(note)}</div>` : ''}${mountable ? '' : prAiButtonHtml(ai, hubView, opts.asking)}`;
}

/* ── the comments ── */

/* The drafts the page holds until the first edit: none. A read-through brings no comments of its own: the person writes them on the lines
   of the diff (a concern's draft becomes one only when 「コメントにする」 is pressed). `fromSha` and `fromMode` say which read-through the page is on. */
function prSeedDrafts(read) {
  return {
    version: 1, fromSha: read?.sha ?? null, ...(read?.mode === 'split' ? { fromMode: 'split' } : {}),
    drafts: [], overall: null,
  };
}

/* Say in `doc` which read-through its comments were last written on: its head, and its mode (`fromMode` is absent for a read-through). */
function prStampDoc(doc, read) {
  if (!read) return doc;
  doc.fromSha = read.sha ?? doc.fromSha ?? null;
  if (read.mode === 'split') doc.fromMode = 'split'; else delete doc.fromMode;
  return doc;
}

/* The first `<prefix><n>` that no draft of `doc` uses. */
function prNextKey(doc, prefix) {
  const have = new Set((doc?.drafts || []).map(d => d.key));
  let n = 1;
  while (have.has(`${prefix}${n}`)) n++;
  return `${prefix}${n}`;
}

function prDraftLocation(d) {
  if (!d?.path) return 'PR 全体';
  if (d.line && d.startLine && d.startLine !== d.line) return `${d.path}:${d.startLine}-${d.line}`;
  return d.line ? `${d.path}:${d.line}` : d.path;
}

/* Every draft with something written in it, then the overall comment: what 「すべてコピー」 puts on the clipboard. */
function prCopyAllText(doc) {
  const parts = (doc?.drafts || []).filter(d => String(d.body || '').trim()).map(d => `${prDraftLocation(d)}\n${d.body}`);
  if (String(doc?.overall?.body || '').trim()) parts.push(`全体のコメント:\n${doc.overall.body}`);
  return parts.join('\n\n');
}

/* A comment as the server compares it: line ends as \n, no trailing space (GitHub may trim it, and what is stored is that). */
const prNormComment = t => String(t ?? '').replace(/\r\n/g, '\n').trimEnd();
/* What 「pending review に積む」 would put in: the drafts with a file and something written that are not in the review yet, and
   whether the overall comment is new (it is not when it is the one already in the review). The server decides the same way. */
function prPendable(doc) {
  const drafts = (doc?.drafts || []).filter(d => d.path && String(d.body || '').trim() && !d.sentAt).length;
  const body = prNormComment(doc?.overall?.body);
  const overall = body !== '' && prNormComment(doc?.pending?.overall) !== body;
  return { drafts, overall };
}

/* What handing the drafts of a task's PR to its worker would send: the drafts with something written that are not sent yet (a draft
   with no file is sent as a point on the whole PR), and whether the overall comment has something written. The server decides the
   same way. */
function prHandable(doc) {
  const drafts = (doc?.drafts || []).filter(d => String(d.body || '').trim() && !d.sentAt).length;
  const overall = String(doc?.overall?.body || '').trim() !== '';
  return { drafts, overall };
}

/* The overall comment as the send dialog shows it: the stored one when the document has an `overall` (even a blank one: it is the
   person's), else the reader's own (for a split proposal, the proposal), until the person types in it or a push writes it into the
   document. `reader` is whether the text is the reader's untouched one. */
function prOverallShown(doc, read) {
  if (doc?.overall) return { text: String(doc.overall.body ?? ''), reader: false };
  const text = String(read?.overall?.body ?? '');
  return { text, reader: text.trim() !== '' };
}
/* `doc` with the overall comment that is shown in it: what the dialog counts and copies is what a push would send, which is what is on screen. */
function prDocShown(doc, read) {
  if (doc?.overall) return doc;
  const { text } = prOverallShown(doc, read);
  return text.trim() === '' ? doc : { ...doc, overall: { body: text } };
}

/* ── the send dialog: 「レビューを送る」 and 「worker に渡す」 (pr-send.js draws it) ── */

/* The overall comment's box, with the reader's verdict hint under it for a review request (`to` 'pending'). The page draws it once when the
   dialog opens, so that what is typed in it is never drawn over; `prSendChipSync` keeps the chip in place. */
function prSendTextHtml(doc, read, to = 'pending') {
  const shown = prOverallShown(doc, read);
  const suggestion = to !== 'worker' && read?.overall?.suggestion
    ? `<div class="pr-suggestion">AI の判定の目安: <b>${esc(read.overall.suggestion)}</b>（文字だけ。判定を選んで送るのは GitHub です）</div>` : '';
  return `<label class="pr-overall-label"><span>全体のコメント <span class="pr-chip" data-pr-send-chip${shown.reader ? '' : ' hidden'}>AI の文から</span></span>`
    + `<textarea class="pr-overall" data-pr-overall aria-label="全体のコメント" spellcheck="false">\n${esc(shown.text)}</textarea></label>${suggestion}`;
}

/* The comments the diff cannot show, as a list of the dialog: where each was meant to be, its text and 「削除」. */
function prSendOffHtml(title, drafts, locked) {
  if (!drafts.length) return '';
  const items = drafts.map(d => `<li><div class="pr-send-off-head"><code class="pr-loc">${esc(prDraftLocation(d))}</code>${d.sentAt ? '<span class="pr-chip pr-chip-sent">送りました</span>' : ''}`
    + `<button type="button" class="btn-m3-text pr-act" data-pr-discard="${esc(d.key)}"${locked ? ' disabled' : ''}>削除</button></div><div class="pr-send-off-text">${esc(d.body)}</div></li>`);
  return `<div class="pr-send-off"><div class="pr-sub">${esc(`${title}（${drafts.length}）`)}</div><ul class="pr-list">${items.join('')}</ul></div>`;
}

/* How the last push into the pending review went, as lines: what it sent, failed and skipped, whether the overall comment went in, and why
   a refused overall was (`result` is the page's `pendResult`: { sent, failed: [{ key, why }], skipped: [{ key, why }], overall }). */
function prPendResultHtml(result) {
  const r = result;
  const out = r ? [`${r.sent || 0} 件を積みました`, r.failed?.length ? `${r.failed.length} 件は積めませんでした` : '', r.skipped?.length ? `${r.skipped.length} 件は対象外でした` : ''].filter(Boolean).join('／') : '';
  const done = [out, r && r.overall && r.overall !== 'none' ? '全体のコメントも積みました' : ''].filter(Boolean).join('／');
  // A refused overall comment has no box of its own to say why under.
  const refused = r?.failed?.find(f => f.key === 'overall');
  return done || refused ? `<div class="pr-pend-result" role="status">${esc(done)}</div>${refused ? `<div class="pr-why" role="note">全体のコメントは積めませんでした: ${esc(refused.why)}</div>` : ''}` : '';
}
/* The same for the last hand-over to a worker (`result`: { sent, overall, present, woken }, or { error }). */
function prHandResultHtml(result) {
  const r = result;
  if (!r) return '';
  if (r.error) return `<div class="pr-why" role="note">${esc(r.error)}</div>`;
  const done = [`${(r.sent || []).length} 件を渡しました`, r.overall ? '全体のコメントも渡しました' : ''].filter(Boolean).join('／');
  return `<div class="pr-pend-result" role="status">${esc(done)}</div>`;
}

/* Everything in the dialog but the overall comment's box: what would be sent, the comments the diff cannot show, how the last send went, the
   link and the line that says how the saving goes (empty here: the page fills it in place, `prShowSave`). `o`: `to` ('pending' or 'worker'),
   `sending`, `result` (`prPendResultHtml`'s or `prHandResultHtml`'s), `prUrl`, `off` and `whole` (`prDiffUnplaced`'s: the drafts with a path the
   diff does not have, and the ones with no path; for a review request both are 差分に出せないコメント, for a worker the second is
   「PR 全体へのコメント」). */
function prSendRestHtml(doc, read, o = {}) {
  const worker = o.to === 'worker';
  const shown = prDocShown(doc, read);
  const n = worker ? prHandable(shown) : prPendable(shown);
  const empty = !n.drafts && !n.overall;
  const note = worker
    ? '<div class="pr-pend-note">worker が差分の確認を待っているときは、その確認を「差し戻す」で答えます。待っていないときは、最新の差分の記録に差し戻しとして渡します。渡したコメントは「渡しました」と印が付き、残ります。</div>'
    : `<div class="pr-pend-note">${esc(`行のコメント ${n.drafts} 件と一緒に、PR の pending review に積みます。承認・変更の依頼は GitHub で選びます。`)}</div>`;
  const why = empty && !o.sending ? `<div class="pr-pend-note">${worker ? '渡せるものがありません。' : '積めるものがありません。'}</div>` : '';
  const locked = !!o.sending;
  const off = worker ? prSendOffHtml('差分に出せないコメント', o.off || [], locked) + prSendOffHtml('PR 全体へのコメント', o.whole || [], locked)
    : prSendOffHtml('差分に出せないコメント', [...(o.whole || []), ...(o.off || [])], locked);
  const url = httpUrl(o.prUrl);
  // The review itself when it is known, else the PR's files, where a pending review can be finished.
  const href = worker ? null : httpUrl(doc?.pending?.url) || (url ? `${url}/files` : null);
  const link = href ? `<div class="pr-pend-note"><a href="${esc(href)}" target="_blank" rel="noopener noreferrer">GitHub で仕上げる</a></div>` : '';
  const result = worker ? prHandResultHtml(o.result) : prPendResultHtml(o.result);
  return `${note}${why}${off}${result}${link}<div class="pr-save" role="status" data-pr-save></div>`;
}

/* The dialog's buttons: 「すべてコピー」, 「閉じる」 and the one that sends (`data-pr-pend`, or `data-pr-hand` for a worker), labelled by what it
   would send; disabled with its title when there is nothing, and while a send is on its way. `o`: `to`, `sending`. */
function prSendFootHtml(doc, read, o = {}) {
  const worker = o.to === 'worker';
  const shown = prDocShown(doc, read);
  const n = worker ? prHandable(shown) : prPendable(shown);
  const empty = !n.drafts && !n.overall;
  const label = worker
    ? (n.drafts ? `${n.drafts} 件を worker に渡す` : '全体のコメントを worker に渡す') + (n.drafts && n.overall ? '（全体のコメントも）' : '')
    : n.drafts ? `${n.drafts} 件を pending review に積む` : n.overall ? '全体のコメントを pending review に積む' : 'pending review に積む';
  const title = empty ? (worker ? '渡せるものがありません。書き込みのあるコメントか、全体のコメントがあると渡せます' : '積めるものがありません。ファイルのあるコメントに、書き込みがあると積めます') : '';
  const busy = o.sending ? ' aria-busy="true"' : '';
  const sendLabel = o.sending ? (worker ? '渡しています…' : '積んでいます…') : empty && worker ? 'worker に渡す' : label;
  return '<button type="button" class="btn-m3-tonal pr-send-copy" data-pr-copy-all>すべてコピー</button>'
    + '<button type="submit" value="cancel" class="btn-m3-text">閉じる</button>'
    + `<button type="button" class="btn-m3-primary" ${worker ? 'data-pr-hand' : 'data-pr-pend'}${empty || o.sending ? ' disabled' : ''}${busy}${title ? ` title="${esc(title)}"` : ''}>${esc(sendLabel)}</button>`;
}

/* ── 「拡大」 ── */

/* The groups of the one view (the middle's 下読み tab, 「拡大」 and a task's own dialog), for `cardDialogTaskHtml`, in this order: 前回のレビューから,
   要点, 見るポイント and 読む順 (a read-through of the older form only), 差分 (not for a split proposal), 設計判断・テスト・書かれていないこと,
   この PR について. A group with no card is left out; before a read-through only 前回のレビューから, 要点 and この PR について can be there.
   The annotated diff's cards (`opts`): `diffFileHtml(path)` for a file, `diffPaths` the files in the order to show
   (the reader's order, then the record's files, until there are any), `diffHeadHtml` a first card (key `diff-bar`) when it is not
   blank, and `diffLabel(path)` the index's `{ label, title }` of a file. The footer is in この PR について, where 「下読みし直す」 can be found. */
function prDialogGroups(rec, read, records, hubView, nowSecs, opts = {}) {
  const secs = prSections(rec, read, records, hubView, nowSecs, opts);
  const of = key => secs.find(s => s.key === key);
  const card = (s, wide = false) => s && { key: s.key, label: s.label, badge: s.badge || '', wide, html: prSecHtml(s), expand: null };
  const group = (key, label, cards, badge = '') => ({ key, label, badge, cards: cards.filter(Boolean) });
  const old = prOldForm(read);
  const groups = [
    group('since', '前回のレビューから', [card(of('since:list'))], of('since:list')?.badge || ''),
    group('top', '要点', [card(of('top:oor')), card(of('top:ai')), card(of('top:old')), card(of('top:three')), card(of('top:split'))]),
    group('points', '見るポイント', old ? [card(of('points:list'), true)] : [], old ? of('points:list')?.badge || '' : ''),
    group('order', '読む順', old ? [card(of('order:list'), true)] : []),
  ];
  // A split proposal did not read the diff: there is none to show.
  if (read && read.mode !== 'split') {
    let paths = opts.diffPaths;
    if (!paths) {
      paths = [];
      for (const p of [...(read.order || []).map(o => o.path), ...(rec?.files || []).map(f => f.path)]) if (p && !paths.includes(p)) paths.push(p);
    }
    const cards = paths.map(path => {
      const stat = prFileStatText(prFileStat(rec, path));
      const html = opts.diffFileHtml ? opts.diffFileHtml(path)
        : `<div class="m3-filled-card pr-sec" data-pr-sec="${esc(`diff:${path}`)}">${secTitle(esc(`${path}${stat ? `（${stat}）` : ''}`))}<div class="tp-muted">${esc(PR_DIFF_PLACEHOLDER)}</div></div>`;
      const named = opts.diffLabel ? opts.diffLabel(path) : null;
      return { key: `diff:${path}`, label: named?.label || baseName(path) || path, title: named?.title || '', badge: '', wide: true, html, expand: null };
    });
    if (opts.diffHeadHtml) cards.unshift({ key: 'diff-bar', label: '差分の概要', title: '', badge: '', wide: true, html: opts.diffHeadHtml, expand: null });
    groups.push(group('diff', '差分', cards, paths.length ? String(paths.length) : ''));
  }
  groups.push(
    group('more', '設計判断・テスト・書かれていないこと', [card(of('more:decisions')), card(of('more:tests')), card(of('more:unwritten'))]),
    group('about', 'この PR について', [card(of('top:facts')), card(of('footer'))]),
  );
  return groups.filter(g => g.cards.length);
}
