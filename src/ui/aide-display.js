/* ── 副官: the pure half of the aide's pane (src/ui/aide.js draws and wires it) ───────────────────
   What the pane holds when nothing is selected in 「いまの仕事」: the aide's terminal on the left and, on the right, the display
   area. These functions read what the server and the work document say and return markup or text, touching neither the page
   nor the network, so `src/ui/tests/aide-display.test.js` runs them under `node --test`. Loaded after util.js (`esc`, `kindOf`)
   and before my-work.js.

   The display area draws one state, whoever produced it:
     { topic: { key, label } | null,
       parts: [{ id, kind, ref, topic, shownAt }],   // every part of `topic`, newest first
       earlier: [{ key, label }] }                   // the topics before it, newest first
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
   answered on this page. A task that is parked is set aside on purpose, so its gate is not a wait. */
function aideWaits(entries) {
  const out = [];
  for (const e of entries || []) {
    if ((e.items || []).some(i => i.kind === 'parked')) continue;
    for (const g of e.gates || []) {
      out.push({ key: g.key, kind: g.gate, since: g.since, nwo: e.nwo, title: e.task?.title || e.gateTitle || g.title || e.ref, taskId: e.task?.id || null });
    }
  }
  // A gate with no time is last, not first: nothing says it has waited.
  const at = w => (Number.isFinite(w.since) ? w.since : Infinity);
  return out.sort((a, b) => (at(a) === at(b) ? 0 : at(a) < at(b) ? -1 : 1));
}

const aidePinSub = waits => (waits.length ? `判断待ち ${waits.length} · 次: ${kindOf(waits[0].kind)[0]} ${waits[0].title}` : '待ちはありません');

/* How many waits are not in what the person last saw in the aide's pane; none before they have seen it. */
const aidePeekCount = (seenKeys, waits) => (seenKeys ? waits.filter(w => !seenKeys.has(w.key)).length : 0);

/* ── what is typed into the aide ── */

/* Text that is typed into the aide's prompt: a control character in it would be a key, so each is a space, and it is cut short. */
const aideCleanText = s => [...String(s ?? '').replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')].slice(0, AIDE_ASK_MAX).join('');

const aideScopeText = nwo => `[${nwo}] `;

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
