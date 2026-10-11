/* ── A PR waiting on your review, in the panel and in 「拡大」: what the read-through says, as markup ─────────
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
/* The state pill's colour, by the class the list's glyph is coloured with (othersStateOf). */
const PR_STATE_PILL = { 'ov-requested': 'pill-warn', 'ov-reading': 'pill-blue', 'ov-ready': 'pill-good', 'ov-pushed': 'pill-purple', 'ov-dim': 'pill-neutral' };
/* The placeholder of a file's card in 「拡大」 where there is no annotated diff hook (a fallback: the page sets one). */
const PR_DIFF_PLACEHOLDER = 'この PR の差分はまだここに表示できません。GitHub で開いて読んでください。';
const PR_NO_READ_WHY = 'AI 下読みのあとに使えます';
const PR_NO_DIFF_WHY = '差分はまだ表示できません';

const prSha7 = sha => String(sha || '').slice(0, 7);
const prSubText = rec => `${rec?.repo || ''} #${rec?.number ?? ''}`;
/* When the stored read-through has to be read again: its head or the time it finished changed. */
const prReadKey = rec => `${rec?.ai?.sha || ''}|${rec?.ai?.finishedAt || ''}`;
/* Whether there is a read-through to show, or one on its way from the server: `read` is undefined while it is being fetched. */
const prHasRead = (rec, read) => !!read || (read === undefined && !!rec?.ai?.sha);

/* The reader's window can be attached to when its record says it is in a tmux window (the middle mounts it, and says so when it cannot). */
const prReaderOpen = rec => ['running', 'ready'].includes(rec?.ai?.status) && rec.ai.terminal?.backend === 'tmux' && !!rec.ai.terminal.window;
/* The boxes the person types a draft in. */
const PR_BOX_SEL = '.pr-draft-body, .pr-overall';
/* What the panel and the middle say while the PR's record is not there: the records are still coming, could not be read, or lack it. */
const prMissingText = (doc, error) => doc ? 'この PR は、いまの一覧にありません' : error ? `読み込めませんでした: ${error}` : '読み込み中…';

/* The tab that is shown: 下読み unless the one named can be used (下書き and 差分 need a read-through; 差分 also needs the
   annotated diff, which says so through `diffOn`). */
function prPaneOf(pane, hasRead, diffOn = false) {
  if (pane === 'drafts' && hasRead) return 'drafts';
  if (pane === 'diff' && hasRead && diffOn) return 'diff';
  return 'detail';
}

/* The panel's tabs. They are not `panelTab`s: those carry the board's `data-pane`, which the task panel's own click
   handler answers. A tab that cannot be used is disabled, with why as its title. */
function prTabsHtml(rec, read, pane, draftCount = 0, diffOn = false) {
  const has = prHasRead(rec, read);
  const tab = (id, label, extra, off) =>
    `<button type="button" role="tab" id="tp-tab-pr-${id}" class="m3-seg-tab${pane === id ? ' active' : ''}" data-pr-pane="${id}" aria-selected="${pane === id}" aria-controls="tp-detail"${off ? ` disabled title="${esc(off)}"` : ''}>${esc(label)}${extra}</button>`;
  const dot = OTHERS_LIVE.includes(rec?.state) && rec?.events?.pushed ? '<span class="tp-dot" role="img" aria-label="前回のレビューのあとに push されました" title="前回のレビューのあとに push されました"></span>' : '';
  const badge = draftCount > 0 ? `<span class="m3-tab-badge" style="background:var(--md-sys-color-surface-container-highest);color:var(--md-sys-color-on-surface);">${draftCount}</span>` : '';
  return tab('detail', '下読み', dot, '')
    + tab('diff', '差分', '', !has ? PR_NO_READ_WHY : !diffOn ? PR_NO_DIFF_WHY : '')
    + tab('drafts', '下書き', badge, !has ? PR_NO_READ_WHY : '');
}

/* The badges and the title of the panel's head; the buttons are added by the caller (prHeadHtml). */
function prHeadMainHtml(rec) {
  const [, stateLabel, stateCls] = othersStateOf(rec);
  const url = httpUrl(rec?.url);
  const chip = url
    ? `<a class="btn-m3-tonal sess-act tp-head-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer" title="GitHub で PR #${esc(rec.number)} を開く"><span class="material-symbols-outlined" aria-hidden="true">merge</span><span>PR #${esc(rec.number)}</span></a>`
    : `<span class="tp-key">PR #${esc(rec?.number)}</span>`;
  const unreg = rec?.registered === false ? ' <span class="ov-unreg" title="adj に登録していないリポジトリです（同じオーナーなので依頼だけ出ます）">未登録</span>' : '';
  return `<div class="tp-head-main">
      <div class="tp-badges">
        ${chip}
        <span class="origin-chip" title="${esc(rec?.repo)}"><span class="material-symbols-outlined" aria-hidden="true">folder</span><span>${esc(rec?.repo)}</span></span>${unreg}
        <span class="m3-pill ${PR_STATE_PILL[stateCls] || 'pill-neutral'}">${esc(stateLabel)}</span>
      </div>
      <h2 class="tp-title">${esc(rec?.title)}</h2>
    </div>`;
}
/* The panel's head: 「GitHub で開く」, 「拡大」 and close (panelBtnsHtml adds the close). */
function prHeadHtml(rec) {
  const url = httpUrl(rec?.url);
  const open = url ? `<a class="btn-m3-text tp-jump" href="${esc(url)}" target="_blank" rel="noopener noreferrer" title="GitHub で開く（新しいタブ）">GitHub で開く</a>` : '';
  const expand = '<button type="button" class="tool-btn" data-pr-expand aria-haspopup="dialog" title="拡大" aria-label="PR 全体を拡大して読む"><span class="material-symbols-outlined" aria-hidden="true">open_in_full</span></button>';
  return prHeadMainHtml(rec) + panelBtnsHtml(open + expand);
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

/* 「前回のレビューから」: for a PR pushed since your review, or one whose read-through compares with it. */
function prSince(rec, read) {
  const since = read?.sinceReview;
  if (rec?.state !== 'pushed' && !since) return null;
  const n = rec?.commitsSinceReview;
  const count = Number.isFinite(n) ? `前回のレビューから +${n} コミット` : '前回のレビューのあとに push されました';
  let html;
  if (since) {
    const comments = (since.comments || []).map(c => prLi(
      `<span class="pr-chip pr-since-${esc(c.status)}">${esc(PR_SINCE_STATUS[c.status] || c.status)}</span> ${prLoc(c.path, c.line)} <span>${esc(c.note)}</span>`));
    const changes = (since.newChanges || []).map(c => prLi(`<code class="pr-loc">${esc(c.path)}</code> <span>${esc(c.note)}</span>`));
    html = `<div class="tp-line">${esc(count)}</div>`
      + (comments.length ? `<ul class="pr-list">${comments.join('')}</ul>` : '<div class="tp-muted">前回の指摘は記録されていません</div>')
      + (changes.length ? `<div class="pr-sub">新しく増えた変更</div><ul class="pr-list">${changes.join('')}</ul>` : '');
  } else {
    html = `<div class="tp-line">${esc(count)}</div><div class="tp-muted">${read ? 'この下読みには前回のレビューとの比較がありません。下読みし直すと入ります。' : 'AI 下読みすると、前回の指摘への対応を確かめます。'}</div>`;
  }
  return { key: 'since:list', label: '前回のレビューから', badge: Number.isFinite(n) ? `+${n}` : '', html };
}

function prThree(read) {
  const t = read?.three;
  if (!t) return null;
  const line = (label, text) => `<div class="pr-three-row"><span class="pr-three-label">${esc(label)}</span><span>${esc(text)}</span></div>`;
  return { key: 'top:three', label: '3行でいうと', html: `${line('目的', t.purpose)}${line('やり方', t.approach)}${line('影響', t.impact)}<p class="pr-essence"><b>この差分の本質:</b> ${esc(t.essence)}</p>` };
}

/* 見るポイント. `opts.diff` is whether 「差分で見る」 can be offered (the annotated diff is there). */
function prPoints(read, opts = {}) {
  const points = read?.points || [];
  if (!points.length) return null;
  const items = points.map((p, i) => {
    const lv = PR_LEVEL[p.level] ? p.level : 'info';
    return `<li class="pr-point pr-lv-${lv}" data-pr-point="${i}">
      <div class="pr-point-head"><span class="pr-level">${esc(PR_LEVEL[p.level] || p.level)}</span>${prLoc(p.path, p.line)}</div>
      <div class="pr-point-title">${esc(p.title)}</div>
      <div class="pr-point-why">${esc(p.why)}</div>
      <div class="pr-point-acts">${opts.diff ? `<button type="button" class="btn-m3-text pr-act" data-pr-goto-diff="${i}">差分で見る</button>` : ''}<button type="button" class="btn-m3-text pr-act" data-pr-ask="${i}">聞く</button><button type="button" class="btn-m3-text pr-act" data-pr-todraft="${i}">下書きに</button></div>
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

/* The foot of the read-through: the commit it was made at, a warning and 「下読みし直す」 when the head has moved, and what a
   read still under way says (`ai` is `othersAiView`'s). */
function prFooter(rec, read, ai, hubView, asking = false) {
  if (!read) return null;
  const stale = !!rec?.ai?.stale;
  const made = read.madeAt ? ` · ${esc(when(read.madeAt))}` : '';
  const under = ai && ['queued', 'running', 'hung', 'failed'].includes(ai.kind) ? `<div class="tp-muted">${esc(ai.text)}</div>` : '';
  const again = stale ? ` <span class="m3-pill pill-warn" title="PR の head が、下読みしたときから動いています">head は ${esc(prSha7(rec.headSha))}</span> <button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '下読みし直す')}</button>` : '';
  return { key: 'footer', label: '下読みの時点',
    html: `<div class="tp-muted">下読みした時点: <code>${esc(prSha7(read.sha))}</code>${made}</div>${again ? `<div class="pr-foot-acts">${again}</div>` : ''}${under}` };
}

/* Before a read-through: the GitHub facts are all there is, and one button. What a read in progress or failed says is
   `othersAiView`'s; `asking` is a request of this page that is not answered yet. Called by the page only when
   `othersOutOfReview` has nothing to say (a later issue draws its own buttons for a PR that is out of review). */
function prAiButtonHtml(ai, hubView, asking) {
  if (!ai?.action) return '';
  return `<button type="button" class="btn-m3-tonal pr-ai" data-pr-ai${prAskAttrs(hubView, asking)}><span class="material-symbols-outlined" aria-hidden="true">menu_book</span><span>${esc(asking ? '依頼中…' : ai.action)}</span></button>`;
}
function prBeforeRead(rec, records, hubView, nowSecs, asking = false) {
  const ai = othersAiView(rec, records, nowSecs);
  const text = ai.kind === 'none' || !ai.text
    ? '下読みするまでは GitHub の情報だけです。AI 下読みすると、要点・見るポイント・読む順・設計判断・テスト範囲・下書きの候補ができます。' : ai.text;
  return { key: 'top:ai', label: 'AI 下読み', html: `<div class="tp-line">${esc(text)}</div>${prAiButtonHtml(ai, hubView, asking)}` };
}

/* The sections of the 下読み tab, in the issue's order. `opts`: `diff` (the annotated diff is there), `asking`, `readError`. */
function prSections(rec, read, records, hubView, nowSecs, opts = {}) {
  const ai = othersAiView(rec, records, nowSecs);
  const hasRead = !!read;
  return [
    prFacts(rec, nowSecs),
    prSince(rec, read),
    !hasRead && read !== undefined && !othersOutOfReview(rec) ? prBeforeRead(rec, records, hubView, nowSecs, opts.asking) : null,
    prThree(read),
    prPoints(read, { diff: opts.diff }),
    prOrder(read, rec, { diff: opts.diff }),
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

/* ── the middle, before there is a terminal in it ── */

function prMiddleHtml(rec, records, hubView, nowSecs, opts = {}) {
  // A PR that is out of review gets its own buttons here, from a later issue.
  if (othersOutOfReview(rec)) return '';
  const ai = othersAiView(rec, records, nowSecs);
  if (opts.ended != null) {
    return '<div class="tp-ph-head">AI 下読みの worker は終わっています</div><div class="tp-muted">下読みはパネルに残っています。</div>'
      + '<button type="button" class="btn-m3-tonal" data-pr-reconnect><span class="material-symbols-outlined" aria-hidden="true">sync</span><span>再接続</span></button>'
      + prAiButtonHtml(ai, hubView, opts.asking);
  }
  if (ai.kind === 'none') {
    return '<div class="tp-ph-head">まだ AI 下読みしていません</div>'
      + '<div class="tp-muted">AI 下読みすると、読むだけの worker がこの PR を先に読みます。そのあと、ここでその worker に差分のことを聞けます。</div>'
      + prAiButtonHtml(ai, hubView, opts.asking);
  }
  const mountable = prReaderOpen(rec);
  const note = mountable ? '接続しています…'
    : ['done', 'stale'].includes(ai.kind) ? '下読みは右のパネルにあります。worker の画面は開けません。'
    : ai.kind === 'running' || ai.kind === 'hung' ? 'worker の画面は開けません（tmux のウィンドウが記録されていません）。' : '';
  return `<div class="tp-ph-head">${esc(ai.text)}</div>${note ? `<div class="tp-muted">${esc(note)}</div>` : ''}${mountable ? '' : prAiButtonHtml(ai, hubView, opts.asking)}`;
}

/* ── the drafts ── */

/* The drafts as the page holds them until the first edit: the reader's, as if they were the file. */
function prSeedDrafts(read) {
  return {
    version: 1, fromSha: read?.sha ?? null,
    drafts: (read?.drafts || []).map((d, i) => ({ key: `r${i + 1}`, source: 'reader', ...d })),
    overall: { body: read?.overall?.body || '' },
  };
}

const prDraftSame = (a, b) => a.path === b.path && (a.line ?? null) === (b.line ?? null) && (a.side || 'right') === (b.side || 'right') && a.body === b.body;

/* The reader's drafts of `read` that `doc` does not hold, with keys that nothing in `doc` uses. After a re-read the person's
   own are never replaced: these are offered to be added. */
function prNewReaderDrafts(doc, read) {
  const have = new Set((doc?.drafts || []).map(d => d.key));
  const out = [];
  (read?.drafts || []).forEach((d, i) => {
    if ((doc?.drafts || []).some(x => prDraftSame(x, d))) return;
    let key = `r${prSha7(read.sha)}-${i + 1}`;
    for (let n = 2; have.has(key); n++) key = `r${prSha7(read.sha)}-${i + 1}-${n}`;
    have.add(key);
    out.push({ key, source: 'reader', ...d });
  });
  return out;
}

/* The first `<prefix><n>` that no draft of `doc` uses. */
function prNextKey(doc, prefix) {
  const have = new Set((doc?.drafts || []).map(d => d.key));
  let n = 1;
  while (have.has(`${prefix}${n}`)) n++;
  return `${prefix}${n}`;
}

/* A point as the person's own draft: a file draft when the point has no line. */
function prDraftFromPoint(point, doc) {
  const d = { key: prNextKey(doc, 'p'), source: 'person', path: point.path };
  if (point.line) d.line = point.line;
  d.side = 'right';
  d.body = `${point.title}: ${point.why}`;
  return d;
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

const PR_SAVE_TEXT = { saved: '保存しました', saving: '保存しています…' };
/* The drafts: one box each, the overall comment, and the buttons. `opts`: `place` ('panel' or 'dock': only the class of the
   wrapper), `notice` (how many drafts a re-read brought that the doc lacks), `save` ({ status, error }). */
function prDraftsHtml(doc, read, opts = {}) {
  const drafts = doc?.drafts || [];
  const items = drafts.map(d => {
    const loc = prDraftLocation(d);
    return `<div class="pr-draft" data-pr-draft="${esc(d.key)}">
      <div class="pr-draft-head"><code class="pr-loc">${esc(loc)}</code>${d.source === 'reader' ? '<span class="pr-chip" title="AI 下読みの下書きです">AI</span>' : ''}<span class="pr-draft-btns"><button type="button" class="btn-m3-text pr-act" data-pr-copy="${esc(d.key)}">コピー</button><button type="button" class="btn-m3-text pr-act" data-pr-discard="${esc(d.key)}">捨てる</button></span></div>
      <textarea class="pr-draft-body" data-pr-body="${esc(d.key)}" aria-label="${esc(`${loc} への下書き`)}" spellcheck="false">
${esc(d.body)}</textarea>
    </div>`;
  });
  const suggestion = read?.overall?.suggestion
    ? `<div class="pr-suggestion">AI の判定の目安: <b>${esc(read.overall.suggestion)}</b>（文字だけ。判定を選んで送るのは GitHub です）</div>` : '';
  const notice = opts.notice > 0
    ? `<div class="pr-notice" role="status">下読みし直した下書きが ${opts.notice} 件あります <button type="button" class="btn-m3-tonal" data-pr-take-new>新しい下書きを足す</button> <button type="button" class="btn-m3-text" data-pr-keep>このままにする</button></div>` : '';
  const save = opts.save?.status === 'error'
    ? `保存できませんでした: ${opts.save.error || ''} 入力はこのページに残っています`
    : PR_SAVE_TEXT[opts.save?.status] || '';
  return `<div class="pr-drafts" data-pr-place="${esc(opts.place || 'panel')}">
    ${notice}
    ${items.length ? items.join('') : '<div class="tp-muted">下書きはまだありません。見るポイントの「下書きに」から足せます。</div>'}
    <label class="pr-overall-label">全体のコメント
      ${suggestion}
      <textarea class="pr-overall" data-pr-overall aria-label="全体のコメント" spellcheck="false">
${esc(doc?.overall?.body || '')}</textarea>
    </label>
    <div class="pr-draft-acts"><button type="button" class="btn-m3-tonal" data-pr-copy-all>すべてコピー</button><div data-pr-pending-slot></div></div>
    <div class="pr-save${opts.save?.status === 'error' ? ' err' : ''}" role="status" data-pr-save>${esc(save)}</div>
  </div>`;
}

/* ── 「拡大」 ── */

/* The groups of the dialog, in the issue's order, for `cardDialogTaskHtml`: 前回のレビューから, 要点, 見るポイント, 読む順,
   設計判断・テスト・書かれていないこと, 差分. A group with no card is left out; before a read-through only the first two
   can be there. The annotated diff's cards (`opts`): `diffFileHtml(path)` for a file, `diffPaths` the files in the order to show
   (the reader's order, then the record's files, until there are any), `diffHeadHtml` a first card (key `diff-bar`), and
   `diffLabel(path)` the index's `{ label, title }` of a file. The footer joins 要点, where 「下読みし直す」 can be found. */
function prDialogGroups(rec, read, records, hubView, nowSecs, opts = {}) {
  const secs = prSections(rec, read, records, hubView, nowSecs, opts);
  const of = key => secs.find(s => s.key === key);
  const card = (s, wide = false) => s && { key: s.key, label: s.label, badge: s.badge || '', wide, html: prSecHtml(s), expand: null };
  const group = (key, label, cards, badge = '') => ({ key, label, badge, cards: cards.filter(Boolean) });
  const groups = [
    group('since', '前回のレビューから', [card(of('since:list'))], of('since:list')?.badge || ''),
    group('top', '要点', [card(of('top:facts')), card(of('top:ai')), card(of('top:three')), card(of('footer'))]),
    group('points', '見るポイント', [card(of('points:list'), true)], of('points:list')?.badge || ''),
    group('order', '読む順', [card(of('order:list'), true)]),
    group('more', '設計判断・テスト・書かれていないこと', [card(of('more:decisions')), card(of('more:tests')), card(of('more:unwritten'))]),
  ];
  if (read) {
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
  return groups.filter(g => g.cards.length);
}
