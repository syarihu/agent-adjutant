/* ── The annotated diff on the page: the 差分 tab of a PR's panel, the 差分 group of 「拡大」 and of a task's read-through ──────────
   pr-diff.js is the pure half (parsing, categories, anchoring, markup); this file holds what the page keeps: the diff of each PR
   as `GET /api/others/diff` answered it (applied only to its own PR and read-through), which category tab and fold are on, which
   files are drawn, and what a click does. It sets `PR_HOOKS.diff`, which pr-panel.js, pr-read.js and pr-dialog.js call. Three places
   draw it: `panel`, `dialog` and `read` (the middle of a task's 「AI 下読み」, which task-read-view.js registers in `PRD_VIEWS`).
   Loaded after pr-dialog.js.

   The markup a hook returns has no time, no draft and no tab or fold state in it (pr-diff.js), because the panel and the dialog
   draw again only when the markup differs. What is on is set in place afterwards (`prdApply`), so a toggle never redraws, and a
   file's lines are put in its body from a cache (`bodies`) rather than drawn again, also after the dialog replaced its content. */

/* A diff of at most this many rows is drawn whole at once; a bigger one file by file, as each comes near the screen. */
const PRD_EAGER_ROWS = 2000;
const PRD_ROOT_MARGIN = '1200px 0px';
/* A diff that could not be read is asked for again after this long (a redraw would otherwise ask again at once). */
const PRD_RETRY_MS = 5000;

const prDiff = {
  cache: new Map(),   // id -> prdNew(): what was fetched for the PR's current read-through
  cat: new Map(),     // id -> the category tab that is on
  folded: false,      // every annotation folded (one switch for the whole page)
  open: new Set(),    // `id|path` of the files that start closed (deleted whole, or very long) that were opened
  scroll: new Map(),  // id -> the panel's scroll top in the 差分 tab
  target: null,       // { id, path, line, at }: where 「拡大」 is to go once the diff is there
  enteredFor: null,   // the PR whose 差分 tab the panel is in (a scroll is saved only then, and only while its diff is drawn)
  restore: false,     // the panel's reading place of `enteredFor` is still to be put back (once its diff is drawn)
  tick: 0,            // counts uses of an entry, to know the least recent
  observers: { panel: null, dialog: null, read: null },
};

function prdNew() {
  const e = { wanted: null, busy: false, retryAt: 0, gen: 0, used: 0 };
  prdReset(e);
  return e;
}
/* Forget what was held: it was for another read-through. `gen` tells a body drawn from the old one from a new one. */
function prdReset(e) {
  Object.assign(e, {
    gen: e.gen + 1, key: null, retryAt: 0, state: 'loading', error: null, bytes: 0, read: null, layout: null, resolved: null, labels: new Map(),
    bodies: new Map(), filled: { panel: new Set(), dialog: new Set(), read: new Set() },
  });
}

/* The diff of the PR's current read-through, asked for when it is not held. States: `loading`, `ok`, `none` (404: this read-through
   has no diff stored), `empty` (the PR has no changes), `big` (over the cap: only its size is stored), `bad` (the text had no file
   in it) and `error`. The answer is applied to the entry of its own PR and only if its read-through is still the one asked for. */
function prDiffEnsure(rec, read) {
  let e = prDiff.cache.get(rec.id);
  if (!e) prDiff.cache.set(rec.id, e = prdNew());
  e.used = ++prDiff.tick;
  prdTrim();
  const key = `${read.sha}|${read.madeAt}`;
  if (e.wanted !== key) { prdReset(e); e.wanted = key; }
  if (e.key === key || e.busy || Date.now() < e.retryAt) return e;
  e.busy = true;
  boardApi('', `/api/others/diff?id=${encodeURIComponent(rec.id)}`).then(res => {
    if (e.wanted !== key) return;
    try {
      prdTake(e, rec, read, res);
      e.key = key;
    } catch (err) {
      prdFailed(e, rec, err);
    }
  }, err => {
    if (e.wanted !== key) return;
    // No diff stored is an answer; any other failure is asked again, after a while.
    if (err.status === 404) { e.state = 'none'; e.error = null; e.key = key; } else prdFailed(e, rec, err);
  }).finally(() => {
    e.busy = false;
    prRedrawFor(rec.id);
  });
  return e;
}

/* A diff that could not be read: said on the page, and asked for again after a while. Nothing else would draw the page again, so
   the retry is scheduled, for the PR that is still the one shown. */
function prdFailed(e, rec, err) {
  e.state = 'error';
  e.error = err.message;
  e.retryAt = Date.now() + PRD_RETRY_MS;
  setTimeout(() => prRedrawFor(rec.id), PRD_RETRY_MS);
}

/* How many PRs other than the selected one are kept: their parsed diffs are small beside their pages, and what was drawn from them
   (`bodies`) is dropped, since it is made again from the lines when the PR is shown again. */
const PRD_KEEP = 3;
function prdTrim() {
  const sel = prShown()?.id ?? null;
  const rest = [...prDiff.cache].filter(([id]) => id !== sel).sort((a, b) => b[1].used - a[1].used);
  rest.slice(PRD_KEEP).forEach(([id]) => { prDiff.cache.delete(id); prDiff.cat.delete(id); prDiff.scroll.delete(id); });
  rest.slice(0, PRD_KEEP).forEach(([, e]) => {
    if (!e.bodies.size) return;
    e.bodies = new Map();
    e.filled = { panel: new Set(), dialog: new Set(), read: new Set() };
    e.gen++;
  });
}

function prdTake(e, rec, read, res) {
  e.error = null;
  e.bytes = Number(res?.bytes) || 0;
  // A diff of another head than the read-through's would put annotations on the wrong lines.
  if (res?.sha !== read.sha) { e.state = 'none'; return; }
  if (res.diff === null) { e.state = 'big'; return; }
  if (res.diff === '') { e.state = 'empty'; return; }
  const files = prParseDiff(res.diff);
  if (!files.length) { e.state = 'bad'; return; }
  e.read = read;
  e.layout = prDiffLayout(files, read, rec);
  e.resolved = prDiffResolve(files, read);
  e.labels = prDiffLabels(e.layout.files.map(f => f.path));
  e.state = 'ok';
}

/* The category tab that is on: the person's, while it has files; else the first file's in the reading order. */
function prdActiveCat(e, id) {
  const want = prDiff.cat.get(id);
  if (want && e.layout.files.some(f => f.cat === want)) return want;
  return e.layout.files[0]?.cat || 'logic';
}

/* ── what the hooks draw ── */

const prdOpenLink = rec => {
  const url = httpUrl(rec?.url);
  return url ? `<a class="btn-m3-text" href="${esc(`${url}/files`)}" target="_blank" rel="noopener noreferrer">GitHub で開く</a>` : '';
};
const prdMb = bytes => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

/* What is said while the diff cannot be drawn: loading, failed, not stored, empty, too large. In 「拡大」 the loading note is the
   one the dialog's change marks know (`DIFF_LOADING`), so the diff arriving is not marked as a change. */
function prdStatusHtml(rec, e, dialog) {
  const card = inner => `<div class="m3-filled-card pr-sec prd-status" data-pr-sec="diff-bar">${inner}</div>`;
  if (e.state === 'loading') return dialog ? DIFF_LOADING : '<div class="tp-muted prd-status" role="status">差分を読み込み中…</div>';
  if (e.state === 'error') return `<div class="wk-notice" role="alert">${esc(`差分を読めませんでした（読み直しています）: ${e.error}`)}</div>`;
  if (e.state === 'empty') return card('<div class="tp-line">差分はありません</div>');
  if (e.state === 'bad') return card('<div class="tp-line">差分を読み取れませんでした。GitHub で開いて読んでください。</div>');
  if (e.state === 'big') return card(`<div class="tp-line">${esc(`差分が大きすぎるため保存していません（${prdMb(e.bytes)}）。GitHub で開いてください`)}</div><div class="pr-foot-acts">${prdOpenLink(rec)}</div>`);
  // 'none': this read-through is from before diffs were kept, or GitHub did not give the diff to the reader.
  const shown = prShown();
  const hubView = shown?.hubView;
  const asking = !!shown?.asking;
  return card('<div class="tp-line">この下読みには差分が保存されていません。下読みし直すと表示されます</div>'
    + `<div class="pr-foot-acts"><button type="button" class="btn-m3-tonal pr-reread" data-pr-reread${prAskAttrs(hubView, asking)}>${esc(asking ? '依頼中…' : '下読みし直す')}</button>${prdOpenLink(rec)}</div>`);
}

/* The stale note, the bar, and the annotations of files the diff does not have. */
function prdBarHtml(rec, read, e) {
  const stale = rec.headSha && read.sha !== rec.headSha
    ? `<div class="tp-muted prd-stale" role="note">${esc(prStaleText(read, rec))}</div>` : '';
  const anns = read.annotations || [];
  const orphans = e.resolved.orphans.length
    ? `<div class="wk-notice" role="note">差分にないファイルの注釈が ${e.resolved.orphans.length} 件あります<ul class="pr-list">${e.resolved.orphans.map(i => `<li><code class="pr-loc">${esc(anns[i].path)}</code> ${esc(anns[i].text)}</li>`).join('')}</ul></div>` : '';
  return `${stale}${prDiffBarHtml(e.layout)}${orphans}`;
}

/* The panel's 差分 tab. */
function prdTabHtml(rec, read) {
  if (!read) return '<div class="tp-muted" role="status">差分を読み込み中…</div>';
  const e = prDiffEnsure(rec, read);
  if (e.state !== 'ok') return prdStatusHtml(rec, e, false);
  return `<div class="prd-root" data-prd-root><div class="m3-filled-card pr-sec">${prdBarHtml(rec, read, e)}${prDiffIndexHtml(e.layout, e.resolved)}</div>`
    + `${e.layout.files.map(f => prDiffShellHtml(f, e.resolved.files.get(f.path))).join('')}</div>`;
}

/* ── the dialog's cards ── */

function prdDialogFileHtml(rec, read, path) {
  const e = prDiff.cache.get(rec.id);
  const f = e?.state === 'ok' && e.layout.files.find(x => x.path === path);
  if (!f) return `<div class="m3-filled-card pr-sec" data-pr-sec="${esc(`diff:${path}`)}">${DIFF_LOADING}</div>`;
  return prDiffShellHtml(f, e.resolved.files.get(path));
}
/* The first card of the group (key `diff-bar`): the bar, or what is said while there is no diff. */
function prdDialogHeadHtml(rec, read) {
  const e = prDiffEnsure(rec, read);
  if (e.state !== 'ok') return prdStatusHtml(rec, e, true);
  return `<div class="m3-filled-card pr-sec" data-pr-sec="diff-bar">${prdBarHtml(rec, read, e)}</div>`;
}
function prdDialogPaths(rec, read) {
  const e = prDiffEnsure(rec, read);
  return e.state === 'ok' ? e.layout.files.map(f => f.path) : [];
}
function prdDialogLabel(rec, path) {
  return prDiff.cache.get(rec.id)?.labels.get(path) || { label: baseName(path) || path, title: path };
}

/* ── putting in place what is on, and drawing the files ── */

/* Where the diff is drawn now: the panel's tab, 「拡大」 when it is open on this PR (or is being filled for opening: `filling`), and
   the middle of a task's 「AI 下読み」. `root` holds the bar and the files, `scroll` is what scrolls them (the observer's root). */
function prdPlace(place, filling = false) {
  if (place === 'panel') {
    const root = tp('tp-rest')?.querySelector('.prd-root');
    return root ? { root, scroll: tp('tp-detail') } : null;
  }
  if (place === 'read') {
    const root = document.getElementById('task-read-content');
    return root?.querySelector('[data-prd-file], [data-prd-tab]') ? { root, scroll: document.getElementById('task-read-body') } : null;
  }
  const dlg = cardDialogEl();
  if (!dlg || !(dlg.open || filling) || cardDialog.mode !== 'review' || cardDialog.subject !== selectedTaskId) return null;
  const root = cardDialogContent();
  return root.querySelector('[data-prd-file], [data-prd-tab]') ? { root, scroll: dlg.querySelector('.card-dialog-body') } : null;
}
function prdCurrent() {
  const id = prShown()?.id;
  if (!id) return null;
  const e = prDiff.cache.get(id);
  return e && e.state === 'ok' && e.key === e.wanted ? { id, e } : null;
}

/* The tab, the fold and the opened files, as attributes of what is drawn: nothing is drawn again. */
function prdApply(id, e, p) {
  const cat = prdActiveCat(e, id);
  p.root.classList.toggle('prd-folded', prDiff.folded);
  prdAskState(p.root, prReaderOpen(prRecOf(id)));
  p.root.querySelectorAll('[data-prd-tab]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.prdTab === cat)));
  p.root.querySelectorAll('[data-prd-fold]').forEach(b => {
    b.setAttribute('aria-pressed', String(prDiff.folded));
  });
  p.root.querySelectorAll('[data-prd-file]').forEach(s => {
    const f = e.layout.files[Number(s.dataset.prdFile)];
    // In 「拡大」 the card is what is hidden, or an empty one is left in the grid.
    (s.closest('.cd-card') || s).hidden = !f || f.cat !== cat;
    if (!f?.closed) return;
    const open = prDiff.open.has(`${id}|${f.path}`);
    const body = s.querySelector('[data-prd-body]');
    if (body) body.hidden = !open;
    s.querySelector('[data-prd-open]')?.toggleAttribute('hidden', open);
  });
}

/* Whether the annotations' 「聞く」 is on: the PR on show has a reader to type to. */
function prdReaderOpen() {
  const cur = prdCurrent();
  return !cur || prReaderOpen(prRecOf(cur.id));
}
/* The annotations' 「聞く」 of what is drawn, set in place: the cached bodies are made once, and the reader can close after. */
function prdAskState(root, open) {
  root.querySelectorAll('[data-prd-ask]').forEach(b => {
    b.disabled = !open;
    if (open) b.removeAttribute('title'); else b.title = PR_ASK_OFF_WHY;
  });
}

/* A file's body, from the cache: its lines are made once for a diff, however often the panel or the dialog is drawn again. */
function prdFillBody(e, place, el) {
  const i = Number(el.dataset.prdBody);
  const f = e.layout.files[i];
  if (!f) return;
  const open = prdReaderOpen();
  if (!e.bodies.has(i)) e.bodies.set(i, prDiffBodyHtml(f, e.resolved.files.get(f.path), e.read, open));
  if (el.dataset.prdFilled !== String(e.gen)) {
    el.innerHTML = e.bodies.get(i);
    el.dataset.prdFilled = String(e.gen);
  }
  prdAskState(el, open);
  e.filled[place].add(i);
}

/* Bring `place` up to date: attributes in place, the files that were filled before filled again (a redraw emptied them), and the
   others filled now when the diff is small (or there is no IntersectionObserver), else as they come near the screen. */
function prdSync(place, filling = false) {
  const cur = prdCurrent();
  const p = cur && prdPlace(place, filling);
  prDiff.observers[place]?.disconnect();
  prDiff.observers[place] = null;
  if (!p) return;
  const { id, e } = cur;
  prdApply(id, e, p);
  const eager = e.layout.rows <= PRD_EAGER_ROWS || typeof IntersectionObserver === 'undefined';
  const later = [];
  p.root.querySelectorAll('[data-prd-body]').forEach(el => {
    if (el.hidden || el.dataset.prdFilled === String(e.gen)) return;
    if (eager || e.filled[place].has(Number(el.dataset.prdBody))) prdFillBody(e, place, el); else later.push(el);
  });
  if (!later.length) return;
  const gen = e.gen;
  const io = new IntersectionObserver(entries => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      io.unobserve(en.target);
      if (e.gen === gen && e.state === 'ok' && en.target.isConnected) prdFillBody(e, place, en.target);
    }
  }, { root: p.scroll, rootMargin: PRD_ROOT_MARGIN });
  later.forEach(el => io.observe(el));
  prDiff.observers[place] = io;
}
const prdSyncAll = () => { prdSync('panel'); prdSync('dialog'); prdSync('read'); };

/* The panel drew its 差分 tab. The PR it is in is noted at once, also while its diff is not there (or will not be), so that a scroll
   of the panel is never taken for the reading place of the PR it was in before. Coming back to the tab, or to a PR in it, puts the
   reading place back, once there is a diff drawn to scroll in; drawing again within it does not. */
function prdOnShown() {
  const id = isPrRef(selectedTaskId) ? prIdOfRef(selectedTaskId) : null;
  if (prDiff.enteredFor !== id) { prDiff.enteredFor = id; prDiff.restore = true; }
  const cur = prdCurrent();
  if (!cur) return;
  prdSync('panel');
  if (prDiff.restore) {
    prDiff.restore = false;
    const saved = prDiff.scroll.get(cur.id);
    if (saved) tp('tp-detail').scrollTop = saved;
  }
}
/* The person scrolled 「拡大」 by hand: a link still waiting for its diff is not theirs any more. */
function prdOnMoved() { prDiff.target = null; }
function prdOnLeft() { prDiff.enteredFor = null; prDiff.restore = false; }
tp('tp-detail').addEventListener('scroll', () => {
  if (prDiff.enteredFor && !prDiff.restore && prdCurrent()?.id === prDiff.enteredFor) prDiff.scroll.set(prDiff.enteredFor, tp('tp-detail').scrollTop);
}, { passive: true });

/* The dialog replaced its content: every file that was filled before is filled again from the cache, before the dialog puts the
   reading position back. */
function prdOnDrawn() { prdOnDrawnAt('dialog'); }
/* The same for any place that replaced its content: the middle of a task's read-through does, as the dialog does. */
function prdOnDrawnAt(place) { prdSync(place, true); }

/* ── going to a place ── */

const prdFlash = el => {
  el.classList.remove('prd-flash');
  void el.offsetWidth;
  el.classList.add('prd-flash');
  setTimeout(() => el.classList.remove('prd-flash'), 1600);
};

/* Open the category of `path`, so that its file is on the page: a link to a file in a tab that is not on goes nowhere. */
function prdShowCategoryOf(path) {
  const cur = prdCurrent();
  const f = cur?.e.layout.files.find(x => x.path === path);
  if (!f) return null;
  prDiff.cat.set(cur.id, f.cat);
  prdSyncAll();
  return f;
}

/* The places a link to a line or a file of the diff can land in, other than the panel's own tab: what each says of itself.
   `ready()` is whether it is open and on the PR, `go(cardKey, groupKey, flash)` brings a card to the top, `cardEl(key)` finds a card
   and `place` is where its files are drawn. The dialog's is here; the middle of a task's read-through sets its own. */
const PRD_VIEWS = {
  dialog: { ready: () => !!cardDialogEl()?.open && cardDialog.mode === 'review', go: (...a) => cardDialogGo(...a), cardEl: key => cardDialogCardEl(key), place: 'dialog' },
};

/* Take the view the PR on show is read in (「拡大」, or the middle of its task's read-through) to the place waiting in
   `prDiff.target`, once the diff is there: the tab first, then the file drawn at once, then the line in the middle. Dropped when
   the diff will not come or the place is not in it, and after a while. */
function prdApplyTarget() {
  const t = prDiff.target;
  if (!t) return;
  const shown = prShown();
  if (Date.now() - t.at > CARD_DIALOG_TARGET_MS || !shown || shown.id !== t.id) { prDiff.target = null; return; }
  const v = PRD_VIEWS[shown.kind === 'task' ? 'read' : 'dialog'];
  if (!v?.ready()) return;
  const e = prDiff.cache.get(t.id);
  if (!e || e.state === 'loading') return;
  prDiff.target = null;
  if (e.state !== 'ok') return void v.go(null, 'diff', true);
  const f = e.layout.files.find(x => x.path === t.path);
  if (!f) return void v.go(null, 'diff', true);
  if (f.closed) prDiff.open.add(`${t.id}|${f.path}`);
  prDiff.cat.set(t.id, f.cat);
  prdSyncAll();
  const card = v.cardEl(`diff:${f.path}`);
  const body = card?.querySelector('[data-prd-body]');
  if (body) prdFillBody(e, v.place, body);
  const k = t.line ? prDiffFindLine(f.file, t.line) : null;
  v.go(`diff:${f.path}`, null, k == null);
  const row = k != null && body?.querySelector(`[data-prd-i="${k}"]`);
  if (row) {
    row.scrollIntoView({ block: 'center' });
    prdFlash(row);
  }
}
cardDialogContent().addEventListener('animationend', ev => ev.target.classList?.remove('prd-flash'));

/* A link to a line or a file: from the panel it opens 「拡大」 at that place; in 「拡大」 and in a task's read-through it goes there. */
function prdGoto(path, line, { place = 'panel', opener = null } = {}) {
  const id = prShown()?.id;
  if (!id) return;
  prDiff.target = { id, path, line: line || null, at: Date.now() };
  if (place === 'panel') openPrDialog(opener);
  prdApplyTarget();
}

/* The index of 「拡大」: the category of a file is turned on before the jump, or the jump lands on a hidden card. */
cardDialogEl().addEventListener('click', e => {
  if (cardDialog.mode !== 'review') return;
  const b = e.target.closest?.('[data-cd-go-card^="diff:"]');
  if (b) prdShowCategoryOf(b.dataset.cdGoCard.slice('diff:'.length));
}, true);

/* ── clicks ── */

/* A draft of the person's own from a line, or from a file when the annotation has no line: as 「下書きに」 of a point does, the
   panel goes to the 下書き tab and the new box takes the keyboard; in 「拡大」 the box appears in the column and the diff stays. */
function prdStartDraft(rec, read, { path, at, body }, inDialog) {
  const id = rec.id;
  if (!prDraftsEntry(id).loaded) {
    note('下書き', true, '下書きを読み込んでいます。少し待ってからもう一度押してください');
    return;
  }
  if (!inDialog && nav.pane !== 'drafts') go({ pane: 'drafts' }, { replace: true });
  const key = prNextKey(prDoc(id, read), 'p');
  if (prEditDrafts(id, read, d => { d.drafts.push(prDiffDraft(d, { path, line: at?.line, side: at?.side, body })); })) {
    prRedrawNow({ sel: `[data-pr-body="${CSS.escape(key)}"]`, inDialog });
  }
}

/* The click of a button of the diff. True when it was one. */
function prdAct(e, { place = 'panel' } = {}) {
  const hit = sel => e.target.closest?.(sel);
  const cur = prdCurrent();
  if (!cur) return false;
  const { id, e: entry } = cur;
  const inDialog = place === 'dialog';
  const rec = prRecOf(id);
  const read = prReadOf(rec) || null;
  let b;
  if ((b = hit('[data-prd-tab]'))) {
    if (!b.disabled) { prDiff.cat.set(id, b.dataset.prdTab); prdSyncAll(); }
    return true;
  }
  if (hit('[data-prd-fold]')) { prDiff.folded = !prDiff.folded; prdSyncAll(); return true; }
  if ((b = hit('[data-prd-open]'))) {
    const f = entry.layout.files[Number(b.dataset.prdOpen)];
    if (f) { prDiff.open.add(`${id}|${f.path}`); prdSyncAll(); }
    return true;
  }
  if ((b = hit('[data-prd-jump]'))) {
    const f = prdShowCategoryOf(entry.layout.files[Number(b.dataset.prdJump)]?.path);
    if (f?.closed) { prDiff.open.add(`${id}|${f.path}`); prdSyncAll(); }
    // The panel's list jumps in the panel; the read-through's in the middle, where its diff is.
    const at = place === 'read' ? 'read' : 'panel';
    const p = f && prdPlace(at);
    if (p) {
      const section = p.root.querySelector(`[data-prd-file="${f.i}"]`);
      const body = section?.querySelector('[data-prd-body]');
      if (body && !body.hidden) prdFillBody(entry, at, body);
      section?.scrollIntoView({ block: 'start' });
    }
    return true;
  }
  // A task's read-through has no drafts to add to: the buttons are not drawn there, and a click that gets here does nothing.
  if (place === 'read' && (hit('[data-prd-plus]') || hit('[data-prd-todraft]'))) return true;
  if ((b = hit('[data-prd-plus]'))) {
    const f = entry.layout.files[Number(b.closest('[data-prd-file]')?.dataset.prdFile)];
    if (f && rec) prdStartDraft(rec, read, { path: f.path, at: prDiffAtOf(b.dataset.prdPlus), body: '' }, inDialog);
    return true;
  }
  if ((b = hit('[data-prd-todraft], [data-prd-ask]'))) {
    const ann = read?.annotations?.[Number(b.closest('[data-prd-ann]')?.dataset.prdAnn)];
    if (!ann || !rec) return true;
    const at = prDiffAtOf(b.dataset.prdAt);
    if (b.matches('[data-prd-todraft]')) prdStartDraft(rec, read, { path: ann.path, at, body: ann.text }, inDialog);
    else {
      // Leaving for the terminal: the dialog goes first, and focus is not given back to the panel.
      if (inDialog) { cardDialog.quiet = true; closeCardDialog(); }
      prAsk({ path: ann.path, line: at?.line, title: String(ann.text).slice(0, 40) });
    }
    return true;
  }
  return false;
}

PR_HOOKS.diff = {
  tabHtml: prdTabHtml, dialogFileHtml: prdDialogFileHtml, goto: prdGoto, onShown: prdOnShown,
  onLeft: prdOnLeft, onMoved: prdOnMoved, onDrawn: prdOnDrawn, onDrawnAt: prdOnDrawnAt, applyTarget: prdApplyTarget, act: prdAct,
  dialogPaths: prdDialogPaths, dialogHeadHtml: prdDialogHeadHtml, dialogLabel: prdDialogLabel,
};
