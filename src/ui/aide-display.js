/* ── 副官: the pure half of the aide's pane (src/ui/aide.js draws and wires it) ───────────────────
   What the pane holds when nothing is selected in 「いまの仕事」: the aide's terminal on the left and, on the right, the display
   area. These functions read what the server and the work document say and return markup or text, touching neither the page
   nor the network, so `src/ui/tests/aide-display.test.js` runs them under `node --test`. Loaded after util.js (`esc`, `kindOf`),
   decide.js (the choices and buttons of a gate), my-work-others.js, my-work-away.js, pr-read.js and pr-diff.js (the words and
   markup of a PR and its diff), and before my-work.js. What a renderer needs
   from the page (a gate's content, its away block, a draft being typed) comes in as `ctx`, never from a global that holds state.

   The display area draws one state, whoever produced it. The aide fills it with its adjutant_show tool and the resident serves it
   at GET /api/aide/display (src/board/aide/display.rs):
     { topic: { key, label } | null,
       parts: [{ id, kind, ref, topic, shownAt }],   // every part of `topic`, newest first; the server adds `by`
       earlier: [{ key, label }] }                   // the topics before it, newest first
   `ref` says what a part is about, by reference only (the page reads the content from the records): `{ board, id }` for a
   gate, a task (or `{ board, parent }` for a parent) and a session; `{ pr }` (or `{ board, id }` of a task) for a PR; `{ pr, point | level }`
   for a read-through; `{ pr | board + id, path, from, to, side }` for lines of a diff; null for the list of waits. `board` and `id` are
   always there (empty when a kind has none). `topic` is the key of the topic and `shownAt` epoch seconds, which a part this page
   makes itself has no use for (null).
   A part is drawn by the renderer its `kind` was registered with (`registerAidePart`); a kind with none is said not to be
   drawable yet, so a part a newer server sends never breaks the pane. */

const AIDE_NOT_STARTABLE = '副官を起動するには、terminal.preset を "tmux" にして、tmux のある環境でサーバーを動かしてください';
/* What 「副官に聞く」 on the head of 新着 asks for. */
const AIDE_ASK_ALL = '新着を上から順に見せて';
/* How long a request may be before it is cut: it is typed into the aide's prompt, not read as a document. */
const AIDE_ASK_MAX = 120;
/* As the Rust side (`is_busy`): a `running` row is believed only while the hooks were heard from this recently. */
const AIDE_RUNNING_TRUSTED_SECS = 600;

const aideIcon = name => `<span class="material-symbols-outlined" aria-hidden="true">${name}</span>`;

/* ── the display area ── */

const aideEmptyDisplay = () => ({ topic: null, parts: [], earlier: [] });

/* A display state with every field the renderers read, whatever came in: a missing array is an empty one. */
function aideDisplayOf(raw) {
  const r = raw && typeof raw === 'object' ? raw : {};
  const list = x => (Array.isArray(x) ? x.filter(y => y && typeof y === 'object') : []);
  const topicOf = t => ({ key: String(t.key ?? ''), label: String(t.label ?? '') });
  return {
    topic: r.topic && typeof r.topic === 'object' ? topicOf(r.topic) : null,
    parts: list(r.parts).map(p => ({ ...p, id: String(p.id ?? ''), kind: String(p.kind ?? '') })),
    earlier: list(r.earlier).map(topicOf),
  };
}

/* While the aide has shown nothing, the display area holds the list of what waits on the person, so it is never empty when
   something does wait. It is made here and not pushed by the server: a topic the server wrote would be one the aide's next
   part clears. As soon as the aide shows anything, its state is drawn as it is. The label is the one the server gives the same
   topic (WAITS_LABEL in src/board/aide/show.rs). */
function aideDisplayWithWaits(display, waits) {
  const d = aideDisplayOf(display);
  if (d.topic || !(waits || []).length) return d;
  return {
    topic: { key: 'waits', label: '判断待ちの一覧' },
    parts: [{ id: 'board-waits', kind: 'waits', ref: null, topic: 'waits', shownAt: null, by: 'board' }],
    earlier: d.earlier,
  };
}

/* A part the page made itself (a gate opened from the list of waits, the next wait after an answer) goes on top of the parts
   the server holds. `local` is `{ part, serverTop, topic }`: `serverTop` is the id of the server's newest part when it was made
   (null when there was none), and it stands only while the server's newest part is still that one. The aide showing something
   new is the aide having the floor again, so the page's part gives way to it. */
const aideLocalHolds = (display, local) => !!local?.part && (aideDisplayOf(display).parts[0]?.id ?? null) === (local.serverTop ?? null);

function aideDisplayFollowed(display, local) {
  const d = aideDisplayOf(display);
  if (!aideLocalHolds(d, local)) return d;
  // With nothing on show the topic is the page's, so that the list of waits does not take the place of the part.
  return { ...d, topic: d.topic || local.topic || { key: 'page', label: '判断待ち' }, parts: [local.part, ...d.parts] };
}

/* The parts the page makes itself. */
const aideGatePart = (board, id) => ({ id: `page-gate:${board}/${id}`, kind: 'gate', ref: { board, id }, topic: 'page', shownAt: null, by: 'board' });
/* A read-through opened from a row of the review requests, as a wait row opens its gate. */
const aideReadPart = pr => ({ id: `page-read:${pr}`, kind: 'read', ref: { board: '', id: '', pr }, topic: 'page', shownAt: null, by: 'board' });
const aideReviewsPart = () => ({ id: 'page-reviews', kind: 'reviews', ref: null, topic: 'page', shownAt: null, by: 'board' });
const AIDE_REVIEWS_TOPIC = { key: 'reviews', label: 'レビュー依頼' };
const aideWaitsPart = () => ({ id: 'page-waits', kind: 'waits', ref: null, topic: 'page', shownAt: null, by: 'board' });
const AIDE_WAITS_TOPIC = { key: 'waits', label: '判断待ちの一覧' };

/* kind -> render(part, ctx) -> { icon, label, where, title, body, cls }. `body` is html the renderer has escaped itself;
   the others are text, escaped here. */
const AIDE_PARTS = new Map();
const registerAidePart = (kind, render) => AIDE_PARTS.set(kind, render);

function aidePartHtml(part, open, ctx = {}) {
  let shown;
  try {
    const render = AIDE_PARTS.get(part.kind);
    shown = render ? render(part, ctx) : null;
  } catch {
    shown = null;
  }
  const s = shown || { icon: 'extension', label: part.kind, where: '', title: '', body: '<div class="tp-muted">この部品はまだ表示できません</div>', cls: '' };
  return `<details class="aide-part ${esc(s.cls || '')}" data-aide-part="${esc(part.id)}"${open ? ' open' : ''}>
    <summary class="aide-part-head"><span class="aide-part-kind">${aideIcon(esc(s.icon || 'extension'))}${esc(s.label || part.kind)}</span><span class="aide-part-where">${esc(s.where || '')}</span><span class="aide-part-title">${esc(s.title || '')}</span></summary>
    <div class="aide-part-body">${s.body || ''}</div></details>`;
}

/* The 新着 line over the display area: how many are new, the first of them, and a way to ask the aide about it. `news` is
   `{ count, first: { title, what, key } }`, or null when nothing is new. */
function aideNewLineHtml(news) {
  if (!news || !(news.count > 0)) return '';
  const f = news.first || {};
  const first = f.title ? ` · <button type="button" class="aide-new-open" data-aide-row="${esc(f.key)}">${esc(f.title)}</button>${f.what ? ` — ${esc(f.what)}` : ''}` : '';
  return `<div class="aide-new">${aideIcon('fiber_new')}<span class="aide-new-text"><b>新着 ${Number(news.count)}</b>${first}</span>`
    + `<button type="button" class="aide-ask-btn" data-aide-ask-new>${aideIcon('support_agent')}<span>副官に聞く</span></button></div>`;
}

/* The button of a part that has a session behind it: it opens that session in this pane, with the way back to the aide. */
const aideTermButtonHtml = rowKey => `<button type="button" class="aide-link" data-aide-row="${esc(rowKey)}">${aideIcon('terminal')}<span>ターミナルを見る</span></button>`;

/* The newest part is open and the rest of its topic is folded to its heading; earlier topics are only named. */
function aideDisplayHtml(display, { news = null, ctx = {} } = {}) {
  const d = aideDisplayOf(display);
  const count = d.parts.length;
  const head = `<div class="aide-display-head">${aideIcon('dashboard_customize')}<span>表示領域</span>${count ? `<span class="aide-display-count">この話題で出したもの ${count}（新しい順）</span>` : ''}</div>`;
  const body = count
    ? `<div class="aide-topic">${aideIcon('label')}<span>${esc(d.topic?.label || '')}</span></div>${d.parts.map((p, i) => aidePartHtml(p, i === 0, ctx)).join('')}`
    : '<div class="aide-display-empty">副官はまだ何も出していません</div>';
  const earlier = d.earlier.length
    ? `<details class="aide-earlier"><summary>${aideIcon('history')}<span>前の話題 ${d.earlier.length}</span></summary><ul>${d.earlier.map(t => `<li>${esc(t.label)}</li>`).join('')}</ul></details>`
    : '';
  return aideNewLineHtml(news) + head + body + earlier;
}

/* ── what waits on the person ── */

/* The open gates of the work document, oldest first. `entries` is `workEntries(doc, isGateDone)`, which leaves out the ones
   answered on this page. A task that is parked is set aside on purpose, so its gate is not a wait. `id` is the gate's ref as
   `gateRef` names it (`<board>/<gate id>`), `board` its board's slug and `gateId` its own id; `entryId` is the entry the gate
   is on and `parent` the key of its task's parent. */
function aideWaits(entries) {
  const out = [];
  for (const e of entries || []) {
    if ((e.items || []).some(i => i.kind === 'parked')) continue;
    for (const g of e.gates || []) {
      const cut = String(g.key).lastIndexOf('/');
      out.push({ key: g.key, id: g.key, board: String(g.key).slice(0, Math.max(cut, 0)), gateId: String(g.key).slice(cut + 1), entryId: e.id,
        parent: e.task?.parent || null, kind: g.gate, since: g.since, nwo: e.nwo, title: e.task?.title || e.gateTitle || g.title || e.ref, taskId: e.task?.id || null });
    }
  }
  // A gate with no time is last, not first: nothing says it has waited.
  const at = w => (Number.isFinite(w.since) ? w.since : Infinity);
  return out.sort((a, b) => (at(a) === at(b) ? 0 : at(a) < at(b) ? -1 : 1));
}

/* The wait to show after `done` (`{ key, nwo, parent, taskId }`, the one just answered) is dealt with: the oldest of `waits`
   (oldest first) in the same repository and the same parent, or with no parent the same task. Never `done` itself. */
function aideNextWait(waits, done) {
  if (!done) return null;
  const same = w => w.nwo === done.nwo && (done.parent ? w.parent === done.parent : !w.parent && !!done.taskId && w.taskId === done.taskId);
  return (waits || []).find(w => w.key !== done.key && same(w)) || null;
}

/* Entries whose latest news is that they finished or failed, newest first: what the person was not there for. A hub does not
   finish. `judged` is `workJudge()`'s map (entry id -> { entry, live }); `titleOf(entryId)` names an entry with no task as the list does. */
function aideFinished(judged, titleOf = () => '') {
  const out = [];
  for (const [id, j] of judged || []) {
    const e = j?.entry;
    if (!e || e.isHub) continue;
    const item = (j.live || []).find(i => i.kind === 'done' || i.kind === 'failed');
    if (!item) continue;
    out.push({ entryId: id, nwo: e.nwo, kind: item.kind, since: item.since, taskId: e.task?.id || null, title: e.task?.title || titleOf(id) || e.gateTitle || e.ref });
  }
  const at = f => (Number.isFinite(f.since) ? f.since : -Infinity);
  return out.sort((a, b) => at(b) - at(a));
}

/* A few lines of a gate's diff as evidence for what it asks: the first hunk of each file a finding names (`location` is
   `path:line`), else the first hunk of the diff. At most `max` lines in all, drawn as the diff view draws them (without its
   buttons). With the PR's read-through `read` the reader's annotations are under their lines, anchored in this diff.
   `{ html, annotated }`: whether an annotation was drawn is what the part says in its heading. */
/* A gate's diff parsed once, and its annotations anchored once for a read-through, not at every redraw. */
/* The gate diffs parsed recently, by their text: a few, so that two parts with different diffs do not parse both at every redraw. */
const AIDE_PARSED_KEPT = 4;
const aideParsed = new Map();
function aideParsedGateDiff(diff) {
  let files = aideParsed.get(diff);
  if (!files) files = prParseDiff(diff).filter(f => f.hunks.length);
  // Taken out and put back last: the one used longest ago is the one let go.
  aideParsed.delete(diff);
  aideParsed.set(diff, files);
  if (aideParsed.size > AIDE_PARSED_KEPT) aideParsed.delete(aideParsed.keys().next().value);
  return files;
}
const AIDE_RESOLVED = new WeakMap();
function aideResolved(files, read) {
  let byRead = AIDE_RESOLVED.get(files);
  if (!byRead) AIDE_RESOLVED.set(files, byRead = new WeakMap());
  if (!byRead.has(read)) byRead.set(read, prDiffResolve(files, read));
  return byRead.get(read);
}
function aideEvidenceParts(diff, findings, read = null, max = 12) {
  const files = aideParsedGateDiff(diff);
  if (!files.length) return { html: '', annotated: false };
  const named = (findings || []).map(f => String(f?.location ?? '').replace(/:\d+(?:-\d+)?$/, '')).filter(Boolean);
  const picked = files.filter(f => named.some(n => n === f.path || n.endsWith(`/${f.path}`) || f.path.endsWith(`/${n}`)));
  const res = read?.annotations?.length ? aideResolved(files, read) : null;
  let left = max;
  let annotated = false;
  const html = [];
  for (const f of picked.length ? picked : files.slice(0, 1)) {
    if (left <= 0) break;
    const ks = f.hunks[0].lines.map(l => l.k).slice(0, left);
    left -= ks.length;
    const r = res?.files.get(f.path) || null;
    if (r && ks.some(k => r.byLine.has(k))) annotated = true;
    html.push(`<div class="aide-ev-file"><code class="pr-loc">${esc(f.path)}</code></div>${prDiffExcerptHtml(f, ks, r, read)}`);
  }
  return { html: html.join(''), annotated };
}
const aideEvidence = (diff, findings, read = null, max = 12) => aideEvidenceParts(diff, findings, read, max).html;

const aidePinSub = waits => (waits.length ? `判断待ち ${waits.length} · 次: ${kindOf(waits[0].kind)[0]} ${waits[0].title}` : '待ちはありません');

/* How many waits are not in what the person last saw in the aide's pane; none before they have seen it. */
const aidePeekCount = (seenKeys, waits) => (seenKeys ? waits.filter(w => !seenKeys.has(w.key)).length : 0);

/* ── what is typed into the aide ── */

/* Text that is typed into the aide's prompt: a control character in it would be a key, so each is a space, and it is cut short. */
const aideCleanText = s => [...String(s ?? '').replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')].slice(0, AIDE_ASK_MAX).join('');

const aideScopeText = nwo => `[${nwo}] `;

/* What the 「この判断」 chip and 「副官に答えを伝える」 type at the start of a message: the gate's ref, which `adjutant_gate_answer`'s procedure reads as
   the gate being talked about (`[gate <board>/<id>] B で`). Typed into the prompt, never sent. */
const aideGateTagText = ref => `[gate ${aideCleanText(ref)}] `;

/* 「[owner/repo] #41「title」の確認待ちについて教えて」. The title gives way when it is too long, so the request still ends as one. */
function aideAskRowText({ nwo, title, taskId, kindLabel }) {
  const id = taskId == null || taskId === '' ? '' : /^\d+$/.test(String(taskId)) ? `#${taskId} ` : `${taskId} `;
  const head = aideCleanText(`${aideScopeText(nwo)}${id}「`);
  const tail = aideCleanText(`」${kindLabel ? `の${kindLabel}` : ''}について教えて`);
  const room = Math.max(AIDE_ASK_MAX - [...head].length - [...tail].length, 0);
  return aideCleanText(head + [...aideCleanText(title)].slice(0, room).join('') + tail);
}

/* ── the header ── */

/* The aide's state in words, the pill's class and whether it is in the middle of something (a reset would stop that). */
function aideStateOf(view, now = Date.now() / 1000) {
  const neutral = word => ({ word, pill: 'pill-neutral', busy: false });
  if (!view) return neutral('読み込み中…');
  switch (view.state) {
    case 'none': return neutral('まだ起動していません');
    case 'starting': return { word: '起動しています…', pill: 'pill-blue', busy: false };
    case 'stopped': return { word: '止まっています', pill: 'pill-err', busy: false };
    case 'running': {
      const a = view.agentSession || {};
      const recent = Number.isFinite(a.lastEventAt) && now - a.lastEventAt < AIDE_RUNNING_TRUSTED_SECS;
      if ((a.status === 'running' && recent) || a.pending != null) return { word: '作業中', pill: 'pill-good', busy: true };
      if (a.status === 'waiting') return { word: '入力待ち', pill: 'pill-warn', busy: false };
      return neutral('待機中');
    }
    default: return neutral('状態不明');
  }
}

function aideHeadHtml(view, { resetting = false, now = Date.now() / 1000 } = {}) {
  const st = aideStateOf(view, now);
  const pct = view?.agentSession?.contextPercent;
  const ctx = pct == null ? null : Math.max(0, Math.min(100, Math.round(Number(pct) || 0)));
  const bar = ctx == null ? '' : `<span class="aide-ctx" title="副官のコンテキスト ${ctx}%">コンテキスト <span class="wk-ctx${ctx >= 90 ? ' crit' : ctx >= 70 ? ' warn' : ''}"><span class="wk-ctx-bar"><i style="width:${ctx}%"></i></span>${ctx}%</span></span>`;
  const notStartable = !!view && !view.startable;
  const off = resetting || !view || notStartable || view.state === 'none' || view.state === 'unknown';
  const title = notStartable ? AIDE_NOT_STARTABLE : '新しいセッションで立て直します（副官は記憶を持たないので、そのまま続けられます）';
  return `<span class="aide-ava" aria-hidden="true">${aideIcon('support_agent')}</span>`
    + '<div class="aide-head-main"><h2 class="aide-title">副官</h2><div class="aide-sub">左で話し、右に副官が出したものが並びます</div></div>'
    + `<span class="m3-pill ${st.pill}">${esc(st.word)}</span>${bar}`
    + `<button type="button" class="btn-m3-text aide-reset" data-aide-reset title="${esc(title)}"${off ? ' disabled' : ''}>${aideIcon('restart_alt')}<span>リセット</span></button>`;
}

/* ── the parts: gate and waits ── */

const AIDE_PLAIN_MAX = 140;
/* The first lines of a text as one short plain line: `lines` of them, cut at `max` characters. */
function aidePlainLine(text, lines = 2, max = AIDE_PLAIN_MAX) {
  const t = mdPlain(text).split('\n').map(l => l.trim()).filter(Boolean).slice(0, lines).join(' ');
  return [...t].length > max ? `${[...t].slice(0, max).join('')}…` : t;
}

/* What the task is, in a line or two: its request, else what the gate says is wrong, else the gate's title. */
const aideTaskLine = (g, task) => aidePlainLine(task?.body) || aidePlainLine(g.problem) || String(g.title || '');

const aideSection = (title, body) => `<div class="aide-gate-sec"><div class="aide-gate-sec-title">${esc(title)}</div>${body}</div>`;

/* The way to the session the gate came from, and the way out when the gate cannot be shown. */
const aideGateTermHtml = got => (got.rowKey ? aideTermButtonHtml(got.rowKey) : '');

const aideGateNoteHtml = (text, got, ref) => `<div class="aide-gate" data-aide-gate="${esc(ref)}"><div class="tp-muted">${esc(text)}</div>`
  + `<div class="aide-gate-actions">${aideGateTermHtml(got || {})}</div></div>`;

/* The body of a gate waiting on the person: what it is, what happened, what is asked, what is unsure, the evidence, the actions.
   The choices, the buttons and the comment box are all inside `data-aide-gate`, so a click finds its gate there. */
function aideGateBodyHtml(got, ref, ctx) {
  const { g, task } = got;
  const draft = ctx.draft ? ctx.draft(ref) : '';
  const waiting = g.wait !== false;
  const parts = [];
  if (got.parentTitle) parts.push(`<div class="aide-gate-parent">${aideIcon('account_tree')}<span>${esc(got.parentTitle)}</span></div>`);
  if (got.refreshError) parts.push(aideStaleNote(got.refreshError));
  parts.push(aideSection('このタスク', `<p class="aide-gate-task">${esc(aideTaskLine(g, task))}</p>`));
  if (got.away) parts.push(aideSection('前回見てから', got.away));
  const ask = g.focus ? `<div class="aide-gate-focus">${md(g.focus)}</div>` : '<div class="tp-muted">確認してほしい点は書かれていません</div>';
  const buttons = (g.options || []).map(o => (BUTTONS[o] || (() => ''))(g)).join('');
  parts.push(aideSection('決めること', `${ask}${choicesHtml(g, waiting, { expand: false })}`
    + `<textarea class="gate-comment" aria-label="コメント" placeholder="修正指示や質問があれば入力してください（選んだ案に下書きがあるときは、空欄ならそれを送ります）...">${esc(draft)}</textarea>`
    + `<div class="decide aide-gate-decide">${buttons}</div>`));
  if (g.unsure) parts.push(aideSection('迷っていること', `<div class="aide-gate-unsure">${md(g.unsure)}</div>`));
  const evidence = aideEvidenceParts(g.diff, g.findings, got.read || null);
  if (evidence.html) {
    const note = evidence.annotated ? `<div class="tp-muted">AI 下読みの注釈（${esc(prSha7(got.read.sha))} 時点）</div>` : '';
    parts.push(aideSection('根拠', `${note}<div class="aide-excerpt aide-gate-diff">${evidence.html}</div>`));
  }
  const target = g.kind === 'plan' ? 'request' : 'gate';
  const tid = task?.id || g.task || '';
  const here = `data-aide-board="${esc(got.board || '')}" data-aide-task="${esc(tid)}"`;
  parts.push(`<div class="aide-gate-actions">`
    + (tid ? `<button type="button" class="aide-link" data-aide-expand ${here} data-aide-target="${esc(target)}" data-aide-gate-ref="${esc(ref)}">${aideIcon('open_in_full')}<span>${g.kind === 'plan' ? '拡大 → 依頼と計画' : '拡大で開く'}</span></button>` : '')
    + aideGateTermHtml(got)
    + (waiting ? `<button type="button" class="aide-link" data-aide-tell-gate="${esc(ref)}" title="入力欄に「[gate …] 」を入れます。続けて「B で」のように書くと、副官が確認してから答えます">${aideIcon('support_agent')}<span>副官に答えを伝える</span></button>` : '')
    + `<button type="button" class="aide-link" data-aide-waits>${aideIcon('list')}<span>一覧へ戻る</span></button>`
    + (task ? `<button type="button" class="aide-link" data-aide-park ${here}>${aideIcon('schedule')}<span>置く</span></button>` : '')
    + (waiting ? `<button type="button" class="aide-link aide-gate-close" data-act="close" title="worker への通知を行わずに、この確認待ちを解決済みとしてアーカイブします">${aideIcon('done_all')}<span>解決済みとして閉じる</span></button>` : '')
    + '</div>');
  return `<div class="aide-gate" data-aide-gate="${esc(ref)}">${parts.join('')}</div>`;
}

/* `ctx.gate(ref)` is `{ g, task, entry, rowKey, away, parentTitle, board, nwo, read, error, gone }` (`read`: the read-through of the task's PR
   when it has one, for the annotations of 「根拠」), or null while the board's content is
   being read. `away` is the timeline of 「離れていた間に」 as markup, `rowKey` the key of the session's row in the list. */
registerAidePart('gate', (part, ctx) => {
  const ref = part.ref ? `${part.ref.board}/${part.ref.id}` : '';
  const got = ctx.gate && ref ? ctx.gate(ref) : null;
  const base = { icon: 'pending_actions', label: '判断待ち', where: got?.nwo || '', title: '', cls: 'wait' };
  if (!got) return { ...base, body: '<div class="tp-muted">読み込み中…</div>' };
  if (got.error) return { ...base, title: got.title || part.ref?.id || '', body: aideGateNoteHtml(`この判断待ちを読めませんでした: ${got.error}`, got, ref) };
  if (got.gone || !got.g || got.g.wait === false) {
    return { ...base, title: got.title || part.ref?.id || '', body: `<div class="tp-muted">この判断待ちは、もう待っていません（答えたか、閉じられました）</div>`
      + '<div class="aide-gate-actions"><button type="button" class="aide-link" data-aide-waits>' + `${aideIcon('list')}<span>判断待ちの一覧へ</span></button></div>` };
  }
  const { g, task } = got;
  const waited = stampSecs(g.openedAt) != null ? ` · ${ago(g.openedAt)}から待ち` : '';
  return { ...base, label: kindOf(g.kind)[0], where: `${got.nwo || ''}${waited}`, title: task?.title || g.title || '', body: aideGateBodyHtml(got, ref, ctx) };
});

/* The first line of what a gate asks, for the list of waits. */
const aideFocusLine = focus => aidePlainLine(focus, 1, 100);

/* The waits grouped by repository, then by parent (those with none last), in the order of the oldest wait of each. */
function aideWaitGroups(waits, parentTitle) {
  const repos = new Map();
  for (const w of waits) {
    if (!repos.has(w.nwo)) repos.set(w.nwo, new Map());
    const parents = repos.get(w.nwo);
    const key = w.parent || '';
    if (!parents.has(key)) parents.set(key, []);
    parents.get(key).push(w);
  }
  return [...repos].map(([nwo, parents]) => ({
    nwo,
    parents: [...parents].sort(([a], [b]) => (a === '' ? 1 : 0) - (b === '' ? 1 : 0))
      .map(([key, list]) => ({ key, title: key ? parentTitle(nwo, key) || key : '親なし', waits: list })),
  }));
}

/* `ctx.waits` the waits (`aideWaits`), `ctx.finished` the finished ones (`aideFinished`), `ctx.parentTitle(nwo, key)`,
   `ctx.repoName(nwo)`, `ctx.focusOf(wait)` `{ state: 'loading' | 'error' | 'gone' | 'ok', focus?, error? }` of the gate on its board, `ctx.rowKey(entryId)`. */
registerAidePart('waits', (part, ctx) => {
  const waits = ctx.waits || [];
  const finished = ctx.finished || [];
  const repoName = ctx.repoName || (x => x);
  const waitHtml = w => {
    const f = ctx.focusOf ? ctx.focusOf(w) : null;
    const said = !f ? '' : f.state === 'loading' ? '読み込み中…' : f.state === 'error' ? '確認内容を読み込めませんでした'
      : f.state === 'gone' ? 'もう待っていないようです' : aideFocusLine(f.focus) || '確認内容は書かれていません';
    const focus = said;
    const since = Number.isFinite(w.since) ? `<span class="aide-wait-when">${esc(sinceLabel(w.since))}から</span>` : '';
    return `<li><button type="button" class="aide-wait" data-aide-open-gate="${esc(w.key)}"><span class="aide-wait-kind">${esc(kindOf(w.kind)[0])}</span>`
      + `<span class="aide-wait-title">${esc(w.title)}</span>${since}${focus ? `<span class="aide-wait-focus">${esc(focus)}</span>` : ''}</button></li>`;
  };
  const groups = aideWaitGroups(waits, ctx.parentTitle || (() => ''));
  const list = groups.length
    ? groups.map(r => `<div class="aide-waits-repo"><div class="aide-waits-repo-name">${esc(repoName(r.nwo))}</div>`
      + r.parents.map(p => `<div class="aide-waits-parent"><div class="aide-waits-parent-title">${aideIcon('account_tree')}<span>${esc(p.title)}</span></div><ul class="aide-waits-list">${p.waits.map(waitHtml).join('')}</ul></div>`).join('')
      + '</div>').join('')
    : '<div class="tp-muted">判断待ちはありません</div>';
  const done = finished.length
    ? `<div class="aide-waits-done"><div class="aide-gate-sec-title">離れている間に終わったこと</div><ul class="aide-waits-list">${finished.map(f => {
      const term = ctx.rowKey && ctx.rowKey(f.entryId) ? aideTermButtonHtml(ctx.rowKey(f.entryId)) : '';
      const when = Number.isFinite(f.since) ? `<span class="aide-wait-when">${esc(sinceLabel(f.since))}</span>` : '';
      return `<li class="aide-finished"><span class="aide-wait-kind${f.kind === 'failed' ? ' bad' : ''}">${f.kind === 'failed' ? '失敗' : '完了'}</span>`
        + `<span class="aide-wait-title">${esc(f.title)}</span><span class="aide-wait-repo">${esc(repoName(f.nwo))}</span>${when}${term}</li>`;
    }).join('')}</ul></div>`
    : '';
  return { icon: 'pending_actions', label: '判断待ち', where: waits.length ? `${waits.length} 件` : '', title: '判断待ちの一覧', cls: 'wait', body: `${list}${done}` };
});

/* ── the parts: read, task, pr, session and diff ──
   Each is drawn from what `ctx` says of the thing the part names, read from the records and never from GitHub. A part whose thing
   is not there yet says so (`読み込み中…`), one that failed says why, and one that is gone says it is gone: never an empty body. */

/* The last read of something failed while an earlier one is still on show: said, so what is drawn is not taken for current. */
const aideStaleNote = err => (err ? `<div class="tp-muted aide-stale">${esc(`最新の読み込みに失敗しました: ${err}（前回読んだ内容を出しています）`)}</div>` : '');
const aideNote = text => `<div class="tp-muted">${esc(text)}</div>`;
const aideLinkBtn = (attrs, icon, text, off = '') => `<button type="button" class="aide-link" ${attrs}${off ? ` disabled title="${esc(off)}"` : ''}>${aideIcon(icon)}<span>${esc(text)}</span></button>`;

/* ── read ── */

/* Which points of a read-through a part opens, by index: the one `ref.point` names (counted from 1), every one of `ref.level`,
   else every one that needs attention, else the first. The rest are folded under the part. */
function aideReadPick(read, ref) {
  const points = read?.points || [];
  const all = points.map((_, i) => i);
  let shown;
  if (ref?.point) shown = points[ref.point - 1] ? [ref.point - 1] : [];
  else if (ref?.level) shown = all.filter(i => points[i].level === ref.level);
  else {
    shown = all.filter(i => points[i].level === 'attention');
    if (!shown.length && points.length) shown = [0];
  }
  return { shown, rest: all.filter(i => !shown.includes(i)) };
}

/* A point's line with three lines of the diff either side of it, within its hunk, and the reader's annotations under them. */
const AIDE_POINT_CONTEXT = 3;
/* Where each point's line is in the parsed diff, found once for a (diff, read-through) and kept with them: a redraw every two seconds
   must not scan a diff of megabytes again for each point. `{ file, k }` per point index, `null` for a point with no line in the diff. */
const AIDE_POINT_PLACES = new WeakMap();
function aidePointPlaces(files, read) {
  let byRead = AIDE_POINT_PLACES.get(files);
  if (!byRead) AIDE_POINT_PLACES.set(files, byRead = new WeakMap());
  let places = byRead.get(read);
  if (!places) {
    const byPath = new Map(files.map(f => [f.path, f]));
    places = (read.points || []).map(p => {
      const file = byPath.get(p.path);
      const k = file && p.line ? prDiffFindLine(file, p.line) : null;
      return k == null ? null : { file, k };
    });
    byRead.set(read, places);
  }
  return places;
}

function aidePointExcerptHtml(p, i, diff, resolved, read) {
  if (!diff || diff.state !== 'ok') return diff?.state === 'loading' ? aideNote('差分を読み込み中…') : '';
  const place = aidePointPlaces(diff.files, read)[i];
  if (!place) return '';
  const { file, k } = place;
  const h = file.all[k].h;
  const ks = [];
  for (let i = Math.max(0, k - AIDE_POINT_CONTEXT); i <= Math.min(file.all.length - 1, k + AIDE_POINT_CONTEXT); i++) if (file.all[i].h === h) ks.push(i);
  return `<div class="aide-excerpt">${prDiffExcerptHtml(file, ks, resolved?.files.get(file.path) || null, read)}</div>`;
}

function aidePointHtml(p, i, diff, resolved, read) {
  const lv = PR_LEVEL[p.level] ? p.level : 'info';
  return `<li class="pr-point pr-lv-${lv} aide-point"><div class="pr-point-head"><span class="pr-level">${esc(PR_LEVEL[p.level] || p.level)}</span>${prLoc(p.path, p.line)}</div>`
    + `<div class="pr-point-title">${esc(p.title)}</div><div class="pr-point-why">${esc(p.why)}</div>${aidePointExcerptHtml(p, i, diff, resolved, read)}</li>`;
}

/* `ctx.pr(prId)` is `{ state: 'loading' | 'error' | 'none' | 'ok', error?, rec, read, diff: { state, files } }`: the record (or null),
   the stored read-through and the parsed stored diff. The buttons carry the PR and, for a task's PR, its board and task. */
registerAidePart('read', (part, ctx) => {
  const shown = aideReadBody(part, ctx);
  // Opened from a row of the review requests: the way back to the list, as a gate has to the list of waits.
  const back = String(part.id).startsWith('page-read:') ? `<div class="aide-gate-actions">${aideLinkBtn('data-aide-reviews', 'list', '一覧へ戻る')}</div>` : '';
  return back ? { ...shown, body: shown.body + back } : shown;
});

function aideReadBody(part, ctx) {
  const id = part.ref?.pr || '';
  const got = ctx.pr && id ? ctx.pr(id) : null;
  const rec = got?.rec || null;
  // The pick is in the heading, so folded parts of one PR are told apart.
  const pick = part.ref?.point ? ` · 見るポイント ${part.ref.point}` : part.ref?.level ? ` · ${PR_LEVEL[part.ref.level] || part.ref.level}のポイント` : '';
  const base = { icon: 'menu_book', label: `AI 下読み${pick}`, where: rec ? prSubText(rec) : id, title: rec?.title || id, cls: '' };
  if (!got || got.state === 'loading') return { ...base, body: aideNote('読み込み中…') };
  if (got.state === 'error') return { ...base, body: aideNote(`この下読みを読めませんでした: ${got.error}`) };
  if (got.state === 'none' || !got.read) return { ...base, body: aideNote('この PR の AI 下読みは、まだありません') };
  const { read, diff } = got;
  const link = rec?.task || null;
  const attrs = `data-aide-pr="${esc(id)}" data-aide-board="${esc(link?.board || '')}" data-aide-task="${esc(link?.id || '')}"`;
  const stale = rec?.headSha && read.sha && rec.headSha !== read.sha ? `<div class="tp-muted">${esc(prStaleText(read, rec))}</div>` : '';
  const parts = [aideStaleNote(got.refreshError), stale];
  const three = prThree(read);
  if (three) parts.push(aideSection('この PR がすること', three.html));
  const split = prSplitProposal(read);
  if (split) parts.push(aideSection('分割案', split.html));
  if (!split) {
    const { shown, rest } = aideReadPick(read, part.ref);
    const resolved = diff?.state === 'ok' ? diff.resolved || (read.annotations?.length ? prDiffResolve(diff.files, read) : null) : null;
    const points = read.points || [];
    if (shown.length) parts.push(aideSection('見るポイント', `<ul class="pr-points aide-points">${shown.map(i => aidePointHtml(points[i], i, diff, resolved, read)).join('')}</ul>`));
    else parts.push(aideSection('見るポイント', aideNote(points.length ? 'この指定に当たるポイントはありません' : 'この下読みには、見るポイントがありません')));
    if (rest.length) {
      const items = rest.map(i => `<li class="pr-point pr-lv-${PR_LEVEL[points[i].level] ? points[i].level : 'info'}"><div class="pr-point-head"><span class="pr-level">${esc(PR_LEVEL[points[i].level] || points[i].level)}</span>${prLoc(points[i].path, points[i].line)}</div><div class="pr-point-title">${esc(points[i].title)}</div></li>`);
      parts.push(`<details class="aide-more"><summary>ほかの見るポイント ${rest.length}</summary><ul class="pr-points aide-points">${items.join('')}</ul></details>`);
    }
  }
  parts.push(`<div class="aide-gate-actions">${aideLinkBtn(`data-aide-open-read ${attrs}`, 'open_in_full', 'AI 下読みを開く')}`
    + `${aideLinkBtn(`data-aide-ask-reader ${attrs}`, 'chat', '読み手に聞く', prReaderOpen(rec) ? '' : PR_ASK_OFF_WHY)}</div>`);
  return { ...base, body: `<div class="aide-read">${parts.join('')}</div>` };
}

/* ── task and parent ── */

/* What happens next to a task, in a line: the person's turn first, then the PR's, then the worker's. */
const AIDE_PR_WAIT = { draft: 'PR は下書きです（worker が仕上げています）', unrequested: 'PR のレビュー待ちです', 'other-reviewer': '他のレビュアーの返事待ちです',
  checks: 'bot・CI の結果待ちです' };
function aideTaskNext(task, gates) {
  if (!task) return '';
  if (parkOf(task)) return `置いています — ${parkText(parkOf(task))}`;
  const open = (gates || []).find(g => g.wait !== false);
  if (open) return `あなたの判断待ち: ${kindOf(open.kind)[0]}`;
  if (task.status === 'done' || task.status === 'cancelled') return 'このタスクは終わっています';
  if (WORK_PR_NOW[task.prTurn]) return `あなたの番: ${WORK_PR_NOW[task.prTurn]}`;
  if (AIDE_PR_WAIT[task.prTurn]) return AIDE_PR_WAIT[task.prTurn];
  // By the status the record says: nothing is on it before a worker is, and once its PR is out it waits on the PR.
  const by = { backlog: 'まだ hub に渡していません（着手されていません）', queued: 'hub が着手するのを待っています', dispatched: 'worker が作業中です', pr: 'PR の結果待ちです' };
  return by[task.status] || '';
}

/* The plan of a task in a line: `plan` is `{ title, state: 'decided' | 'waiting', at, text }` (`at` a stamp). */
function aidePlanLine(plan, finished = false) {
  // The server leaves a finished task's plan off its card: that is not the same as there being none.
  if (!plan) return finished ? '終わったタスクの計画はここでは読めません' : '計画はまだありません';
  const when = plan.at && stampSecs(plan.at) != null ? ` · ${ago(plan.at)}` : '';
  const said = aidePlainLine(plan.text, 1, 100);
  return `${plan.title || '計画'} — ${plan.state === 'decided' ? '決定済み' : 'あなたの判断待ち'}${when}${said ? `: ${said}` : ''}`;
}

/* The bucket of a child of a parent, the numbers of the bar, and what each child waits on. `children` are the parent's (`{ id, hub,
   title, progress }`), `waitsByChild` the open waits (`aideWaits`) and `tasksByChild` the work document's task of each child
   (`{ prTurn, waitsOnPerson, ... }`), both by `hub/id`. A child the tracker counts and the board does not list is not started. */
const AIDE_BUCKET_WORD = { merged: 'マージ済み', done: '完了', waiting: 'あなた待ち', working: '進行中', 'not-started': '未着手' };
const AIDE_PERSON_TURNS = ['changes', 'merge', 'ci-failed', 'closed'];
const aideChildKey = c => `${c.hub}/${c.id}`;
function aideChildBucket(child, waits, task) {
  // `done` is a task that finished with no merged PR: counted apart from `merged`, as the server keeps them apart.
  if (child.progress === 'merged') return 'merged';
  if (child.progress === 'done') return 'done';
  if ((waits || []).length || task?.waitsOnPerson || AIDE_PERSON_TURNS.includes(task?.prTurn)) return 'waiting';
  if (child.progress === 'pr' || child.progress === 'working') return 'working';
  return 'not-started';
}
/* Whose turn a child is, in words. */
function aideChildTurn(child, waits, task, hasRow) {
  const first = (waits || [])[0];
  if (first) return `あなた（${kindOf(first.kind)[0]}）`;
  if (task?.waitsOnPerson || AIDE_PERSON_TURNS.includes(task?.prTurn)) return `あなた（${WORK_PR_NOW[task?.prTurn] || 'PR'}）`;
  if (child.progress === 'merged' || child.progress === 'done') return '';
  if (task?.prTurn === 'unrequested' || task?.prTurn === 'other-reviewer') return 'レビュアー';
  if (task?.prTurn === 'checks') return 'bot・CI';
  // A task not handed over, or handed to the hub and not picked up, has no worker yet.
  if (['backlog', 'queued'].includes(task?.status) && child.progress !== 'working' && child.progress !== 'pr') return 'hub（着手待ち）';
  if (child.progress === 'working' || child.progress === 'pr' || hasRow || task) return 'worker';
  return 'hub（着手待ち）';
}
function aideParentBuckets(children, waitsByChild, tasksByChild, rowsByChild, total = 0) {
  const counts = { merged: 0, done: 0, waiting: 0, working: 0, notStarted: 0 };
  const items = (children || []).map(c => {
    const waits = waitsByChild?.get(aideChildKey(c)) || [];
    const task = tasksByChild?.get(aideChildKey(c)) || null;
    const bucket = aideChildBucket(c, waits, task);
    counts[bucket === 'not-started' ? 'notStarted' : bucket]++;
    return { id: c.id, hub: c.hub, title: c.title || `#${c.id}`, bucket, turn: aideChildTurn(c, waits, task, !!rowsByChild?.get(aideChildKey(c))),
      waitKey: waits[0]?.key || null, rowKey: rowsByChild?.get(aideChildKey(c)) || null };
  });
  const all = Math.max(total || 0, items.length);
  counts.notStarted += all - items.length;
  return { items, counts, total: all };
}

/* 「次に起きること」 of a parent: its oldest wait, else the first child not started in stack order, else that it is done. */
function aideParentNext(items, waits, total = 0) {
  const wait = (waits || [])[0];
  if (wait) return `あなたの判断待ち: ${wait.title}`;
  const next = (items || []).find(i => i.bucket === 'not-started');
  if (next) return `次に着手: ${next.title}`;
  // The ones the tracker counts and the board has no record of are not started, as the bar draws them.
  const unrecorded = Math.max((total || 0) - (items || []).length, 0);
  if (unrecorded) return `次に着手: まだ記録のない子 ${unrecorded} 件`;
  if ((items || []).length && items.every(i => i.bucket === 'merged')) return 'すべてマージ済み';
  if ((items || []).length && items.every(i => i.bucket === 'merged' || i.bucket === 'done')) return 'すべて終わっています（マージされていないものがあります）';
  return 'worker が進めています';
}

const aideBarHtml = (items, total) => {
  const segs = items.map(i => `<i class="${i.bucket === 'waiting' ? 'waiting' : i.bucket}" title="${esc(`${i.title} — ${AIDE_BUCKET_WORD[i.bucket]}`)}"></i>`);
  for (let n = items.length; n < Math.min(total, 60); n++) segs.push('<i class="not-started" title="未着手"></i>');
  return `<span class="wk-bar aide-bar" role="img" aria-label="${esc(`${items.length} 件 / 全 ${total} 件`)}">${segs.join('')}</span>`;
};

/* `ctx.task(board, id)` is `{ task, plan, gates, nwo, next, rowKey, error, gone }` or null while the board is being read;
   `ctx.parent(board, key)` is `{ p, children, counts, total, next, error, gone }` or null. */
registerAidePart('task', (part, ctx) => {
  const ref = part.ref || {};
  const isParent = !!ref.parent;
  const got = ctx[isParent ? 'parent' : 'task']?.(ref.board, isParent ? ref.parent : ref.id) ?? null;
  const base = { icon: isParent ? 'account_tree' : 'task_alt', label: isParent ? '親 Issue' : 'タスク', where: got?.nwo || '', title: got?.title || (isParent ? ref.parent : ref.id) || '', cls: '' };
  if (!got) return { ...base, body: aideNote('読み込み中…') };
  if (got.error) return { ...base, body: aideNote(`読めませんでした: ${got.error}`) };
  if (got.gone) return { ...base, body: aideNote(isParent ? 'この親 Issue は、もうボードにありません' : 'このタスクは、もうボードにありません') };
  if (isParent) {
    const { p, children, counts, total } = got;
    const list = children.map(c => {
      const open = c.waitKey ? aideLinkBtn(`data-aide-open-gate="${esc(c.waitKey)}"`, 'pending_actions', '判断を見る')
        : c.rowKey ? aideTermButtonHtml(c.rowKey) : '';
      return `<li class="aide-child ${esc(c.bucket)}"><span class="aide-child-title">${esc(c.title)}</span><span class="aide-child-bucket">${esc(AIDE_BUCKET_WORD[c.bucket])}</span>`
        + `${c.turn ? `<span class="aide-child-turn">${esc(c.turn)}</span>` : ''}${open}</li>`;
    }).join('');
    const body = aideSection('進み具合', `<div class="aide-bar-row">${aideBarHtml(children, total)}<span>${esc(`マージ ${counts.merged} / ${counts.done ? `完了 ${counts.done} / ` : ''}あなた待ち ${counts.waiting} / 進行中 ${counts.working} / 未着手 ${counts.notStarted}（全 ${total}）`)}</span></div>`)
      + (children.length ? aideSection('子タスク', `<ul class="aide-children">${list}</ul>`) : '')
      + aideSection('次に起きること', `<p class="aide-gate-task">${esc(got.next || '')}</p>`)
      + (httpUrl(p?.url) ? `<div class="aide-gate-actions"><a class="aide-link" href="${esc(httpUrl(p.url))}" target="_blank" rel="noopener noreferrer">${aideIcon('open_in_new')}<span>GitHub で開く</span></a></div>` : '');
    return { ...base, where: got.nwo || '', title: `${p?.number ? `#${p.number} ` : ''}${p?.title || ref.parent}`, body: `<div class="aide-task">${aideStaleNote(got.refreshError)}${body}</div>` };
  }
  const { task, plan, gates } = got;
  const here = `data-aide-board="${esc(ref.board)}" data-aide-task="${esc(task.id)}"`;
  const pr = task.pr ? ` · PR ${esc(task.pr)}` : '';
  const body = aideSection('このタスク', `<p class="aide-gate-task">${esc(aidePlainLine(task.body) || task.title || task.id)}</p><div class="tp-muted">${esc(`#${task.id} · ${task.status || ''}`)}${pr}</div>`)
    + aideSection('計画', `<p class="aide-gate-task">${esc(aidePlanLine(plan, ['done', 'cancelled'].includes(task.status)))}</p>`)
    + aideSection('次に起きること', `<p class="aide-gate-task">${esc(got.next ?? aideTaskNext(task, gates))}</p>`)
    + `<div class="aide-gate-actions">${aideLinkBtn(`data-aide-expand ${here} data-aide-target="request"`, 'open_in_full', '拡大')}${got.rowKey ? aideTermButtonHtml(got.rowKey) : ''}</div>`;
  return { ...base, title: task.title || task.id, body: `<div class="aide-task">${aideStaleNote(got.refreshError)}${body}</div>` };
});

/* ── request ── */

/* How far a request has got, from the task's own record: a list of `{ key, state, text }` in the order things happen. `state` is `done`,
   `now` (the step it is at), `todo`, or `failed`, which stands for the end of the line (it could not start, or it was cancelled) and
   leaves no step at `now`. `gates` are the task's gates; an open dispatch gate is the hub asking the person to confirm what it read. */
function aideRequestSteps(task, gates = []) {
  const t = task || {};
  const open = (gates || []).find(g => g.kind === 'dispatch' && g.wait !== false);
  const issue = String(t.issue || t.issueUrl || '').trim();
  const number = (issue.match(/(\d+)\s*$/) || [])[1];
  const filed = !!issue;
  // Not `worktree` alone: a hub at its worker limit keeps the worktree and puts the task back to queued.
  const started = ['dispatched', 'pr', 'done'].includes(t.status);
  const reason = String(t.note || '').startsWith('Could not start:') ? String(t.note).slice('Could not start:'.length).trim() : null;
  const cancelled = t.status === 'cancelled';
  const read = !t.needsReading || filed || started;
  const flags = [
    { key: 'sent', done: true, text: '送信済み' },
    { key: 'confirm', done: read && !open, text: open ? '読み取りの確認待ち（あなたの判断）' : read ? '読み取りの確認済み' : 'hub が読むのを待っています' },
    { key: 'issue', done: filed, text: filed ? `起票済み${number ? ` #${number}` : ''}` : '起票' },
    { key: 'worker', done: started, text: 'worker 着手' },
    { key: 'pr', done: !!t.pr || t.status === 'pr', text: t.pr ? `PR ${t.pr}` : 'PR' },
  ];
  const ended = reason != null || cancelled;
  let marked = false;
  const steps = flags.map(f => {
    if (f.done) return { key: f.key, state: 'done', text: f.text };
    if (!marked && !ended) { marked = true; return { key: f.key, state: 'now', text: f.text }; }
    return { key: f.key, state: 'todo', text: f.text };
  });
  // Finished with no PR (a report or a check): nothing is left to wait for, so no step is at `now` and the line ends in 完了.
  if (t.status === 'done' && !t.pr && !ended) {
    for (const st of steps) if (st.state === 'now') st.state = 'todo';
    steps.push({ key: 'done', state: 'done', text: '完了' });
  }
  if (reason != null) steps.push({ key: 'failed', state: 'failed', text: `着手できず${reason ? `: ${reason}` : ''}` });
  else if (cancelled) steps.push({ key: 'failed', state: 'failed', text: '取り消されました' });
  return steps;
}

/* Said in words for a screen reader: the mark beside a step is an icon it does not read. */
const AIDE_STEP_STATE = { done: '完了: ', now: '現在: ', todo: '未着手: ', failed: '失敗: ' };

const aideStepsHtml = steps => `<ol class="aide-steps">${steps.map(s => `<li class="aide-step ${esc(s.state)}" data-state="${esc(s.state)}">`
  + `<span class="aide-step-mark" aria-hidden="true">${aideIcon(s.state === 'done' ? 'check_circle' : s.state === 'now' ? 'pending' : s.state === 'failed' ? 'error' : 'radio_button_unchecked')}</span>`
  + `<span class="aide-step-state">${esc(AIDE_STEP_STATE[s.state] || '')}</span>`
  + `<span class="aide-step-text">${esc(s.text)}</span></li>`).join('')}</ol>`;

/* `ctx.request(board, id)` is `{ task, gates, nwo, hubLabel, hubPresent, hubRowKey, rowKey, waitKey, refreshError, error, gone }` or null while the
   board is being read. `hubRowKey` is the hub's row in the list, `rowKey` the worker's, `waitKey` the open gate of the task (`<board>/<gate id>`). */
registerAidePart('request', (part, ctx) => {
  const ref = part.ref || {};
  const got = ctx.request ? ctx.request(ref.board, ref.id) : null;
  const base = { icon: 'send', label: '依頼', where: got?.nwo || '', title: got?.title || ref.id || '', cls: '' };
  if (!got) return { ...base, body: aideNote('読み込み中…') };
  if (got.error) return { ...base, body: aideNote(`読めませんでした: ${got.error}`) };
  if (got.gone || !got.task) return { ...base, body: aideNote('この依頼のタスクは、もうボードにありません') };
  const { task, gates } = got;
  const here = `data-aide-board="${esc(ref.board)}" data-aide-task="${esc(task.id)}"`;
  const sentTo = `${got.hubLabel || 'hub'} · ${got.nwo || ''}`;
  const stopped = got.hubPresent === false ? '<div class="tp-muted aide-hub-down">hub は止まっています。起動すると読みます</div>' : '';
  const body = aideSection('送った内容', `<p class="aide-gate-task aide-request-body">${esc(String(task.body || '').trim() || task.title || task.id)}</p>`)
    + aideSection('送り先', `<p class="aide-gate-task">${esc(sentTo)}</p>${stopped}`)
    + aideSection('進み具合', aideStepsHtml(aideRequestSteps(task, gates)))
    + `<div class="aide-gate-actions">`
    + (got.waitKey ? aideLinkBtn(`data-aide-open-gate="${esc(got.waitKey)}"`, 'pending_actions', '判断を見る') : '')
    + (got.hubRowKey ? aideLinkBtn(`data-aide-row="${esc(got.hubRowKey)}"`, 'terminal', 'hub のターミナル') : '')
    + (got.rowKey ? aideLinkBtn(`data-aide-row="${esc(got.rowKey)}"`, 'terminal', 'worker のターミナル') : '')
    + aideLinkBtn(`data-aide-expand ${here} data-aide-target="request"`, 'open_in_full', '拡大')
    + '</div>';
  return { ...base, title: task.title || task.id, body: `<div class="aide-task aide-request">${aideStaleNote(got.refreshError)}${body}</div>` };
});

/* ── pr ── */

/* A login that is a review bot: GitHub writes an app's login with `[bot]` in a webhook but leaves it off in the GraphQL answer the
   records come from (`copilot-pull-request-reviewer`), so the names of the usual ones are matched as well. */
const aideIsBot = login => /\[bot\]$|(^|[-_])bot$|^(copilot|coderabbit|github-actions|dependabot|renovate|gemini-code-assist|codex|sonar|codecov)/i.test(String(login || ''));

/* Whose turn a record alone says the PR is, as `PrTurn` would, when no task says. */
function aideTurnOfRecord(rec) {
  if (!rec) return null;
  if (rec.prState === 'merged') return 'merged';
  if (rec.prState === 'closed') return 'closed';
  const rs = rec.reviewers || [];
  const ci = rec.ci || {};
  if (rec.draft) return 'draft';
  if (rs.some(r => r.state === 'changes-requested')) return 'changes';
  if (Number(ci.fail) > 0) return 'ci-failed';
  if (Number(ci.pending) > 0) return 'checks';
  return rs.some(r => r.state === 'approved') ? 'merge' : 'unrequested';
}

/* What stands between the PR and a merge, by the records: `{ ok, blockers }`. A conflict is not in them. */
/* Whose turn a task's PR is from what the task's last refresh read of it (`prStatus`), when no live card says: a finished task has no `prTurn`. */
function aideTurnOfStatus(ps) {
  if (!ps) return null;
  // As `task::pr_turn` reads it: the state first, then changes, a failed check, a running one, approval, a review still owed.
  if (ps.state === 'merged' || ps.state === 'closed') return ps.state;
  if (ps.state === 'draft') return 'draft';
  if (ps.state !== 'open') return null;
  if (ps.review === 'changes') return 'changes';
  if (Number(ps.ci?.fail) > 0) return 'ci-failed';
  if (Number(ps.ci?.pending) > 0) return 'checks';
  if (ps.review === 'approved') return 'merge';
  return ps.review === 'required' ? 'other-reviewer' : 'unrequested';
}

function aideMergeBlockers(turn, ci) {
  if (turn === 'merged') return { ok: false, merged: true, blockers: [] };
  const blockers = [];
  if (turn === 'closed') blockers.push('マージされずに閉じられています');
  if (turn === 'draft') blockers.push('下書きです');
  if (turn === 'changes') blockers.push('修正の依頼があります');
  if (turn === 'ci-failed' || Number(ci?.fail) > 0) blockers.push('CI が落ちています');
  if (turn === 'checks' || Number(ci?.pending) > 0) blockers.push('CI が実行中です');
  if (turn === 'unrequested' || turn === 'other-reviewer') blockers.push('レビュー待ちです');
  return { ok: turn === 'merge' && !blockers.length, blockers };
}

const AIDE_OTHERS_STATE = { requested: 'あなたの番（レビューを頼まれています）', 'ai-reading': 'AI 下読み中', 'ai-ready': 'あなたの番（下読み済み）', pushed: 'あなたの番（push があったので見直し）',
  'waiting-on-author': '作者の番（あなたのレビュー済み）', done: '終わっています' };
const AIDE_TURN_WORD = { ...AIDE_PR_WAIT, merged: 'マージ済みです', closed: 'マージされずに閉じられました', changes: '修正の依頼が来ています（worker の番）', merge: '承認済みです（あなたがマージする番）', 'ci-failed': 'CI が落ちています' };

/* `ctx.prFacts(ref)` is `{ rec, task, prId, prStatus, prTurn, phase, nowSecs, error, loading }`: the record of the PR (null when it has
   none), the task it is of, what the task's last refresh read of it, the turn derived from that, and the worker's phase. */
registerAidePart('pr', (part, ctx) => {
  const ref = part.ref || {};
  const got = ctx.prFacts ? ctx.prFacts(ref) : null;
  const base = { icon: 'merge_type', label: 'PR', where: got?.rec ? prSubText(got.rec) : got?.prId || '', title: got?.prStatus?.title || got?.rec?.title || got?.task?.title || ref.pr || '', cls: '' };
  if (!got || got.loading) return { ...base, body: aideNote('読み込み中…') };
  if (got.error) return { ...base, body: aideNote(`この PR を読めませんでした: ${got.error}`) };
  const { rec, task, prStatus, phase } = got;
  if (!rec && !prStatus) return { ...base, body: aideNote('この PR は読めません（記録がありません）') };
  // The task's own read of its PR is newer than the record's, which only a sync or a read-through rewrites: it comes first, and says
  // merged or closed whatever the record still holds.
  const done = prStatus && ['merged', 'closed'].includes(prStatus.state) ? prStatus.state : null;
  const turn = done || got.prTurn || (prStatus ? aideTurnOfStatus(prStatus) : aideTurnOfRecord(rec));
  const ci = prStatus?.ci || rec?.ci || {};
  const rows = [];
  rows.push(prKv('レビュー（誰の番）', esc(task || !rec ? AIDE_TURN_WORD[turn] || '記録がありません' : AIDE_OTHERS_STATE[rec.state] || AIDE_TURN_WORD[turn] || '')));
  const bots = (rec?.reviewers || []).filter(r => aideIsBot(r.login));
  const botText = bots.map(r => `${esc(r.login)} ${esc(PR_REVIEW_STATE[r.state] || r.state || '')}`.trim());
  if (phase === 'pr-bots') botText.push('worker が bot の指摘待ち');
  rows.push(prKv('レビュー bot', botText.length ? botText.join('<br>') : rec ? 'なし' : '記録がありません'));
  const ciKnown = [ci.pass, ci.fail, ci.pending].some(n => Number(n) > 0);
  rows.push(prKv('CI', esc(ciKnown ? `成功 ${Number(ci.pass) || 0} / 失敗 ${Number(ci.fail) || 0} / 実行中 ${Number(ci.pending) || 0}` : 'なし')));
  const m = aideMergeBlockers(turn, ci);
  // The records say nothing of a conflict or of a branch rule, so "can merge" is never said: only what they do show.
  const merge = m.merged ? 'マージ済みです' : m.ok ? '承認済み・CI 成功' : `まだです — ${m.blockers.join('、') || '承認がまだです'}`;
  rows.push(prKv('マージ', `${esc(merge)}${m.merged ? '' : '<div class="tp-muted">コンフリクトは記録にありません（GitHub で確認）</div>'}`));
  rows.push(prKv('差分の大きさ', rec ? `<span class="add">+${Number(rec.additions) || 0}</span> <span class="del">−${Number(rec.deletions) || 0}</span> · ${Number(rec.changedFiles) || 0} files · ${esc(othersSize(rec))}` : '記録がありません'));
  const ai = rec ? othersAiView(rec, [], got.nowSecs) : null;
  const sha = rec?.ai?.sha ? prSha7(rec.ai.sha) : '';
  const mine = rec?.myReview ? `<div class="tp-muted">あなたのレビュー: ${esc(PR_REVIEW_STATE[rec.myReview.state] || rec.myReview.state)} · ${esc(ago(rec.myReview.submittedAt))}</div>` : '';
  const readText = !rec ? '記録がありません' : ai.kind === 'none' ? 'まだ読んでいません' : `${ai.text}${ai.kind === 'done' && sha ? `（${sha} 時点）` : ''}`;
  rows.push(prKv('読んだか', `${esc(readText)}${mine}`));
  const url = httpUrl(rec?.url || task?.pr);
  const attrs = rec ? `data-aide-pr="${esc(rec.id)}" data-aide-board="${esc(rec.task?.board || '')}" data-aide-task="${esc(rec.task?.id || '')}"` : '';
  const acts = (url ? `<a class="aide-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer">${aideIcon('open_in_new')}<span>GitHub で開く</span></a>` : '')
    + (rec?.ai ? aideLinkBtn(`data-aide-open-read ${attrs}`, 'menu_book', 'AI 下読みを開く') : '');
  return { ...base, body: `<div class="aide-pr">${aideStaleNote(got.refreshError)}<div class="tp-kvs">${rows.join('')}</div>${acts ? `<div class="aide-gate-actions">${acts}</div>` : ''}</div>` };
});

/* ── session ── */

const AIDE_STOPPED = ['stopped', 'failed', 'ended', 'unknown', 'none'];
/* 「止まっていないか」: `{ stuck, text }`. `why` is what is known of a session that is not running. */
function aideStuckLine({ st, label, why, phaseMinutes, stuckAfter }) {
  if (AIDE_STOPPED.includes(st)) return { stuck: true, text: `${label}${why ? ` — ${why}` : ''}` };
  if (st === 'permission') return { stuck: false, text: '入力待ち（許可や答えを待っています）' };
  if (st === 'working' && stuckAfter > 0 && phaseMinutes != null && phaseMinutes >= stuckAfter) return { stuck: true, text: `${minutesLabel(phaseMinutes)} 同じ工程` };
  return { stuck: false, text: '止まっていません' };
}

/* `ctx.session(board, id)` is `{ r, fns, stuckAfter }`, `{ gone: true }` or null while the work document is not read. `r` is a row of the
   list (`workListRows`); `fns` has what draws its state: `glyph(st)`, `label(st)`, `why(r)`, `request(s)`, `said(text)`, `phaseMinutes(r)`. */
registerAidePart('session', (part, ctx) => {
  const ref = part.ref || {};
  const got = ctx.session ? ctx.session(ref.board, ref.id) : null;
  const base = { icon: 'terminal', label: 'セッション', where: got?.nwo || '', title: ref.id || '', cls: '' };
  if (!got) return { ...base, body: aideNote('読み込み中…') };
  if (got.gone) return { ...base, body: aideNote('このセッションは、いまの一覧にありません（終わったか、消えました）') };
  const { r, fns } = got;
  const { s, st, task } = r;
  const a = s.present ? s.agentSession || {} : {};
  const agent = [s.agent, a.model, a.contextPercent != null ? `コンテキスト ${Math.round(Number(a.contextPercent) || 0)}%` : ''].filter(Boolean).join(' · ');
  const label = fns.label(st);
  const stuck = aideStuckLine({ st, label, why: fns.why(r), phaseMinutes: fns.phaseMinutes(r), stuckAfter: got.stuckAfter });
  const rows = [
    prKv('いま', `${fns.glyph(st)} ${esc(label)}${a.activity && st === 'working' ? ` · ${esc(a.activity)}` : ''}`),
    prKv('エージェント', esc(agent || '分かりません')),
    prKv('ブランチ・PR', esc([s.branch, task?.pr ? `PR ${task.pr}` : s.branchPr?.number ? `PR #${s.branchPr.number}` : ''].filter(Boolean).join(' · ') || 'なし')),
    prKv('止まっていないか', `<span class="${stuck.stuck ? 'aide-stuck' : ''}">${esc(stuck.text)}</span>`),
  ];
  if (a.request && st === 'permission') rows.push(prKv('求めていること', esc(fns.request(s))));
  if (s.waiting) rows.push(prKv('判断待ち', esc(`${kindOf(s.waiting.kind)[0]}（あなたの判定待ち）`)));
  const said = a.lastMessage
    ? `<details class="aide-more"><summary>最後のメッセージ${fns.said(a.lastMessage) ? ` — ${esc(fns.said(a.lastMessage))}` : ''}</summary><div class="aide-said">${md(a.lastMessage)}</div></details>` : '';
  return { ...base, where: got.nwo || '', title: task?.title || fns.title?.(r) || ref.id, body: `<div class="aide-session">${aideStaleNote(got.refreshError)}<div class="tp-kvs">${rows.join('')}</div>${said}`
    + `<div class="aide-gate-actions">${r.key ? aideTermButtonHtml(r.key) : ''}</div></div>` };
});

/* ── diff ── */

/* `ctx.diffOf(ref)` is `{ state: 'loading' | 'error' | 'gone' | 'ok', error?, file, res, read, board, taskId, gateRef }`: the file of the parsed
   source, the reader's annotations anchored in it (for a PR's diff) and the read-through. */
registerAidePart('diff', (part, ctx) => {
  const ref = part.ref || {};
  const got = ctx.diffOf ? ctx.diffOf(ref) : null;
  const range = `${ref.path || ''}:${ref.from}${ref.to !== ref.from ? `-${ref.to}` : ''}${ref.side === 'left' ? '（削除された側）' : ''}`;
  const base = { icon: 'difference', label: '差分', where: '', title: range, cls: '' };
  if (!got || got.state === 'loading') return { ...base, body: aideNote('読み込み中…') };
  if (got.state === 'error') return { ...base, body: aideNote(`差分を読めませんでした: ${got.error}`) };
  if (got.state === 'gone') return { ...base, body: aideNote(ref.pr ? 'この PR の差分は、もう読めません' : 'この判断待ちの差分はもう読めません') };
  const ex = got.file ? prDiffExcerpt(got.file, { from: ref.from, to: ref.to, side: ref.side || 'right' }) : null;
  const body = ex ? `<div class="aide-excerpt">${prDiffExcerptHtml(got.file, ex.ks, got.res || null, got.read || null)}</div>` : aideNote('この範囲は差分にありません');
  const act = ref.pr
    ? aideLinkBtn(`data-aide-open-read data-aide-pr="${esc(ref.pr)}" data-aide-board="${esc(got.board || '')}" data-aide-task="${esc(got.taskId || '')}"`, 'menu_book', 'AI 下読みを開く')
    : got.taskId ? aideLinkBtn(`data-aide-expand data-aide-board="${esc(got.board || '')}" data-aide-task="${esc(got.taskId)}" data-aide-target="gate" data-aide-gate-ref="${esc(got.gateRef || '')}"`, 'open_in_full', '拡大で開く') : '';
  return { ...base, body: `<div class="aide-diff">${aideStaleNote(got.refreshError)}${body}${act ? `<div class="aide-gate-actions">${act}</div>` : ''}</div>` };
});

/* ── reviews: the code reviews the person is asked to do ── */

/* `ctx.reviews()` is `{ state: 'loading' | 'error' | 'ok', error?, records, marks, nowSecs, refreshError }`: the records of 「AI 下読み」 that wait on
   the person (the page's own list, read from the resident's records), the read marks of this browser and the server's clock. A row is a
   button: with a read-through it opens it in the area, else the PR in 「AI 下読み」. */
registerAidePart('reviews', (part, ctx) => {
  const got = ctx.reviews ? ctx.reviews() : null;
  const base = { icon: 'rate_review', label: 'レビュー依頼', where: '', title: 'AI 下読みの一覧', cls: 'wait' };
  if (!got || got.state === 'loading') return { ...base, body: aideNote('読み込み中…') };
  if (got.state === 'error') return { ...base, body: aideNote(`レビュー依頼を読めませんでした: ${got.error}`) };
  const waiting = (got.records || []).filter(r => OTHERS_LIVE.includes(r.state));
  // When the list was last synced with GitHub (the person asks for a sync), and what it could not do: without it a list that was never
  // synced, or only in part, would read as no review requests.
  const sy = got.sync;
  const syncHtml = !sy ? '' : `<div class="tp-muted aide-sync${sy.stale || sy.never ? ' warn' : ''}">${esc(sy.never ? 'まだ同期していません（同期するまで、レビュー依頼は出ません）' : sy.words)}</div>`
    + (sy.notes || []).map(n => `<div class="tp-muted aide-sync${n.alert ? ' warn' : ''}">${esc(n.text)}</div>`).join('');
  const stale = aideStaleNote(got.refreshError) + syncHtml;
  // Only a clean, fresh sync may say there are none: a sync that never ran, failed, was partial or is old may have missed some.
  const doubtful = !!sy && (sy.never || sy.stale || (sy.notes || []).length > 0);
  const none = sy?.never ? '同期するまで、レビュー依頼があるかは分かりません' : doubtful ? '同期に問題があるので、レビュー依頼が漏れているかもしれません' : 'レビュー依頼はありません';
  if (!waiting.length) return { ...base, body: `${stale}${aideNote(none)}` };
  const row = r => {
    const flags = [othersSeenClass(r, got.marks?.[r.id] || {}) === 'new' ? '新着' : '', r.rerequest ? '頼み直し' : ''].filter(Boolean).join('・') || '依頼';
    const ai = othersAiView(r, got.records, got.nowSecs);
    const read = ai.kind === 'none' ? 'AI 下読みはまだです' : ai.text;
    return `<li><button type="button" class="aide-wait" data-aide-open-review="${esc(r.id)}"><span class="aide-wait-kind">${esc(flags)}</span>`
      + `<span class="aide-wait-title">${esc(`#${r.number} ${r.title}`)}</span><span class="aide-wait-when">${esc(r.author ? `@${r.author}` : '')}</span>`
      + `<span class="aide-wait-focus">${esc(read)}</span></button></li>`;
  };
  const groups = othersGroups(waiting, '', '', got.nowSecs);
  const body = groups.map(g => `<div class="aide-waits-repo"><div class="aide-waits-repo-name">${esc(g.label)}</div><ul class="aide-waits-list">${g.rows.map(row).join('')}</ul></div>`).join('');
  return { ...base, where: `${waiting.length} 件`, body: `${stale}${body}` };
});
