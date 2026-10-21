/* ── The annotated diff on the page: the 差分 group of 「拡大」 (a review request's, and a task's own AI 下読み dialog) and of the read-through in the middle (a review request's) ──
   pr-diff.js is the pure half (parsing, categories, anchoring, markup); this file holds what the page keeps: the diff of each PR
   as `GET /api/others/diff` answered it (applied only to its own PR and read-through), which closed files were opened, which
   files are drawn, the person's comments on their lines, and what a click does. It sets `PR_HOOKS.diff`, which pr-panel.js, pr-read.js and pr-dialog.js call. Two places
   draw it: `dialog` and `read` (the middle, which task-read-view.js registers in `PRD_VIEWS`).
   Loaded after pr-dialog.js.

   The markup a hook returns has no time, no comment and no open-file state in it (pr-diff.js), because the middle and the dialog
   draw again only when the markup differs. What is on is set in place afterwards (`prdApply`), so opening a file never redraws, and a
   file's lines are put in its body from a cache (`bodies`) rather than drawn again, also after the dialog replaced its content. The
   comments are put in the same way (`prdComments`): they are the drafts' document and the editors being typed in (`prView.boxes`) made
   into elements on their lines, and an element that is being typed in is never replaced. */

/* A diff of at most this many rows is drawn whole at once; a bigger one file by file, as each comes near the screen. */
const PRD_EAGER_ROWS = 2000;
const PRD_ROOT_MARGIN = '1200px 0px';
/* A diff that could not be read is asked for again after this long (a redraw would otherwise ask again at once). */
const PRD_RETRY_MS = 5000;

const prDiff = {
  cache: new Map(),   // id -> prdNew(): what was fetched for the PR's current read-through
  open: new Set(),    // `id|path` of the files that start closed (deleted whole, or very long) that were opened
  target: null,       // { id, path, line, at }: where 「拡大」 is to go once the diff is there
  tick: 0,            // counts uses of an entry, to know the least recent
  observers: { dialog: null, read: null },
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
    bodies: new Map(), filled: { dialog: new Set(), read: new Set() },
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
  rest.slice(PRD_KEEP).forEach(([id]) => { prDiff.cache.delete(id); });
  rest.slice(0, PRD_KEEP).forEach(([, e]) => {
    if (!e.bodies.size) return;
    e.bodies = new Map();
    e.filled = { dialog: new Set(), read: new Set() };
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

/* ── what the hooks draw ── */

const prdOpenLink = rec => {
  const url = httpUrl(rec?.url);
  return url ? `<a class="btn-m3-text" href="${esc(`${url}/files`)}" target="_blank" rel="noopener noreferrer">GitHub で開く</a>` : '';
};
const prdMb = bytes => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

/* What is said while the diff cannot be drawn: loading, failed, not stored, empty, too large. The loading note is the one the
   dialog's change marks know (`DIFF_LOADING`), so the diff arriving is not marked as a change. */
function prdStatusHtml(rec, e) {
  const card = inner => `<div class="m3-filled-card pr-sec prd-status" data-pr-sec="diff-bar">${inner}</div>`;
  if (e.state === 'loading') return DIFF_LOADING;
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

/* The stale note, the note on files the diff lacks, and the annotations of files the diff does not have. Blank when there is none. */
function prdBarHtml(rec, read, e) {
  const stale = rec.headSha && read.sha !== rec.headSha
    ? `<div class="tp-muted prd-stale" role="note">${esc(prStaleText(read, rec))}</div>` : '';
  const anns = read.annotations || [];
  const orphans = e.resolved.orphans.length
    ? `<div class="wk-notice" role="note">差分にないファイルの注釈が ${e.resolved.orphans.length} 件あります<ul class="pr-list">${e.resolved.orphans.map(i => `<li><code class="pr-loc">${esc(anns[i].path)}</code> ${esc(anns[i].text)}</li>`).join('')}</ul></div>` : '';
  return `${stale}${prDiffBarHtml(e.layout)}${orphans}`;
}

/* ── the dialog's cards ── */

function prdDialogFileHtml(rec, read, path) {
  const e = prDiff.cache.get(rec.id);
  const f = e?.state === 'ok' && e.layout.files.find(x => x.path === path);
  if (!f) return `<div class="m3-filled-card pr-sec" data-pr-sec="${esc(`diff:${path}`)}">${DIFF_LOADING}</div>`;
  return prDiffShellHtml(f, e.resolved.files.get(path));
}
/* The first card of the group (key `diff-bar`): the bar when it has something to say, or what is said while there is no diff.
   Blank (no card) when the diff is there and the bar has nothing to say. */
function prdDialogHeadHtml(rec, read) {
  const e = prDiffEnsure(rec, read);
  if (e.state !== 'ok') return prdStatusHtml(rec, e);
  const bar = prdBarHtml(rec, read, e);
  return bar ? `<div class="m3-filled-card pr-sec" data-pr-sec="diff-bar">${bar}</div>` : '';
}
function prdDialogPaths(rec, read) {
  const e = prDiffEnsure(rec, read);
  return e.state === 'ok' ? e.layout.files.map(f => f.path) : [];
}
function prdDialogLabel(rec, path) {
  return prDiff.cache.get(rec.id)?.labels.get(path) || { label: baseName(path) || path, title: path };
}

/* ── putting in place what is on, and drawing the files ── */

/* Where the diff is drawn now: 「拡大」 when it is open on this PR (or is being filled for opening: `filling`), and the middle
   (a review request's 下読み tab). `root` holds the bar and the files, `scroll` is what scrolls them (the observer's root). */
function prdPlace(place, filling = false) {
  if (place === 'read') {
    // Kept while the other tab or another selection is on, but not on screen: what another PR puts in place must not reach it.
    if (document.getElementById('task-read')?.hidden !== false) return null;
    const root = document.getElementById('task-read-content');
    return root?.querySelector('[data-prd-file]') ? { root, scroll: document.getElementById('task-read-body') } : null;
  }
  const dlg = cardDialogEl();
  if (!dlg || !(dlg.open || filling) || !cardDialogOnPr() || cardDialog.subject !== selectedTaskId) return null;
  const root = cardDialogContent();
  return root.querySelector('[data-prd-file]') ? { root, scroll: dlg.querySelector('.card-dialog-body') } : null;
}
function prdCurrent() {
  const id = prShown()?.id;
  if (!id) return null;
  const e = prDiff.cache.get(id);
  return e && e.state === 'ok' && e.key === e.wanted ? { id, e } : null;
}

/* The opened files, as attributes of what is drawn: nothing is drawn again. Every file's card is shown. */
function prdApply(id, e, p) {
  prdAskState(p.root, prReaderOpen(prRecOf(id)));
  p.root.querySelectorAll('[data-prd-file]').forEach(s => {
    const f = e.layout.files[Number(s.dataset.prdFile)];
    // In 「拡大」 the card is what is hidden, or an empty one is left in the grid.
    (s.closest('.cd-card') || s).hidden = !f;
    if (!f?.closed) return;
    const open = prDiff.open.has(`${id}|${f.path}`);
    const body = s.querySelector('[data-prd-body]');
    if (body) body.hidden = !open;
    s.querySelector('[data-prd-open]')?.toggleAttribute('hidden', open);
  });
  prdComments(id, e, p.root);
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

/* A file's body, from the cache: its lines are made once for a diff, however often the middle or the dialog is drawn again. */
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
  // The comments of its lines, which the body's markup does not have.
  const root = place === 'dialog' ? cardDialogContent() : document.getElementById('task-read-content');
  const id = prShown()?.id;
  if (root && id) prdComments(id, e, root, i);
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
const prdSyncAll = () => { prdSync('dialog'); prdSync('read'); };

/* The person scrolled 「拡大」 by hand: a link still waiting for its diff is not theirs any more. */
function prdOnMoved() { prDiff.target = null; }

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

/* The places a link to a line or a file of the diff can land in: what each says of itself.
   `ready()` is whether it is open and on the PR, `go(cardKey, groupKey, flash)` brings a card to the top, `cardEl(key)` finds a card
   and `place` is where its files are drawn. The dialog's is here; the middle sets its own. */
const PRD_VIEWS = {
  dialog: { ready: () => !!cardDialogEl()?.open && cardDialogOnPr(), go: (...a) => cardDialogGo(...a), cardEl: key => cardDialogCardEl(key), place: 'dialog' },
};

/* Take the view the PR on show is read in (「拡大」 when it is open on it, else the middle) to the place waiting in
   `prDiff.target`, once the diff is there: the file drawn at once, then the line in the middle. Dropped when
   the diff will not come or the place is not in it, and after a while. */
function prdApplyTarget() {
  const t = prDiff.target;
  if (!t) return;
  const shown = prShown();
  if (Date.now() - t.at > CARD_DIALOG_TARGET_MS || !shown || shown.id !== t.id) { prDiff.target = null; return; }
  const v = PRD_VIEWS.dialog.ready() && cardDialog.subject === selectedTaskId ? PRD_VIEWS.dialog : PRD_VIEWS.read;
  if (!v?.ready()) return;
  const e = prDiff.cache.get(t.id);
  if (!e || e.state === 'loading') return;
  prDiff.target = null;
  if (e.state !== 'ok') return void v.go(null, 'diff', true);
  const f = e.layout.files.find(x => x.path === t.path);
  if (!f) return void v.go(null, 'diff', true);
  if (f.closed) prDiff.open.add(`${t.id}|${f.path}`);
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

/* A link to a line or a file: in 「拡大」 and in the middle it goes there. */
function prdGoto(path, line, { place = 'read' } = {}) {
  const id = prShown()?.id;
  if (!id) return;
  prDiff.target = { id, path, line: line || null, at: Date.now() };
  prdApplyTarget();
}

/* ── clicks ── */

/* ── the person's comments on the lines ── */

/* Whether `el` is being typed in: a comment's box with the keyboard. Such an element is never replaced or drawn again. */
const prdTyping = el => !!el && !!document.activeElement && el.contains(document.activeElement) && document.activeElement.matches?.('textarea');
/* The state of an element's buttons and box, set in place since it changes without the markup: everything is off while the comments are being put
   into a pending review or handed to a worker, and 「コメントを足す」 and 「保存」 while the box is blank. */
function prdCmtControls(el, locked) {
  const text = el.querySelector('textarea')?.value ?? '';
  el.querySelectorAll('button, textarea').forEach(c => {
    c.disabled = locked || (c.matches('[data-prd-cmt-act="add"], [data-prd-cmt-act="save"]') && !text.trim());
  });
}
/* 「コメントにする」 of a concern is hidden while a comment from the reader already sits on its line, so that one concern is not added twice. */
function prdConcernState(scope, e, doc) {
  scope.querySelectorAll('.prd-ann-concern [data-prd-todraft]').forEach(b => {
    const at = prDiffAtOf(b.dataset.prdAt);
    const f = e.layout.files[Number(b.closest('[data-prd-file]')?.dataset.prdFile)];
    const has = !!at && !!f && (doc?.drafts || []).some(d => d.source === 'reader' && d.path === f.path && (d.side === 'left' ? 'left' : 'right') === at.side && d.line === at.line);
    b.toggleAttribute('hidden', has);
  });
}

/* Make the comment elements of `root` match the drafts' document and the open editors: add the ones that are missing, take away those whose comment is
   gone or moved, and draw again those whose text or state changed. A line's comments sit under its annotations and the comments of the last
   review, a file's in the slot under its header. An element whose box has the keyboard is left as it is, and the files whose body is not filled yet
   get theirs when they are (`prdFillBody`). `only` is the index of the one file to bring up to date (the rest of `root` is not looked at). */
function prdComments(id, e, root, only = null) {
  if (!root || e.state !== 'ok') return;
  const entry = prDraftsEntry(id);
  const doc = prDoc(id, e.read);
  const boxes = prView.boxes.get(id);
  const shown = prShown();
  const failed = new Map((entry.pendResult?.failed || []).map(f => [f.key, f.why]));
  // One file's comments are made from that file's drafts and editors only: a diff of many files is filled file by file.
  const file = only == null ? null : e.layout.files[only];
  const mine = only == null ? doc : { ...doc, drafts: (doc.drafts || []).filter(d => d.path === file?.path) };
  const mineBoxes = only == null || !boxes ? boxes : new Map([...boxes].filter(([, b]) => b.kind !== 'new' || b.path === file?.path));
  const items = prdCommentItems(e.layout.files, mine, mineBoxes, { readSha: e.read?.sha, to: shown?.id === id && shown.kind === 'task' ? 'worker' : 'pending', failed })
    .filter(x => only == null || x.i === only);
  const locked = !!(entry.pending || entry.handing);
  const scope = only == null ? root : root.querySelector(`[data-prd-file="${only}"]`);
  if (!scope) return;
  const have = new Map();
  scope.querySelectorAll('[data-prd-cmt]').forEach(el => have.set(el.dataset.prdCmt, el));
  const cards = new Map();
  const card = i => {
    if (!cards.has(i)) cards.set(i, only === i ? scope : root.querySelector(`[data-prd-file="${i}"]`));
    return cards.get(i);
  };
  const placed = new Set();
  // Where the last comment of a line (or of a file's slot) went, so that the next one follows it.
  const tail = new Map();
  for (const item of items) {
    const sec = card(item.i);
    const body = sec?.querySelector('[data-prd-body]');
    let anchor;
    let slot = null;
    if (item.k != null) {
      // The line is drawn only once its file's body is filled.
      if (!body || body.dataset.prdFilled !== String(e.gen)) continue;
      const row = body.querySelector(`[data-prd-i="${item.k}"]`);
      if (!row) continue;
      anchor = row;
      while (anchor.nextElementSibling?.classList?.contains('prd-ann')) anchor = anchor.nextElementSibling;
    } else {
      slot = sec?.querySelector('[data-prd-fcmts]');
      if (!slot) continue;
    }
    const place = `${item.i}:${item.k ?? 'file'}`;
    let el = have.get(item.key);
    // A box being typed in is kept while it is still that editor; one that an action closed (保存, コメントを足す, やめる) is replaced even with
    // the keyboard in it, since a browser that does not move focus to a pressed button leaves it there.
    const keep = !!el && prdTyping(el) && ['new', 'edit'].includes(item.kind) && el.dataset.prdKind === item.kind;
    if (el && el.dataset.prdSig !== item.sig && !keep) {
      const act = el.contains(document.activeElement) ? document.activeElement.dataset?.prdCmtAct : undefined;
      const fresh = prdCmtEl(item);
      el.replaceWith(fresh);
      el = fresh;
      if (act !== undefined) (el.querySelector(`[data-prd-cmt-act="${act}"]`) || el.querySelector('textarea, button'))?.focus({ preventScroll: true });
    } else if (!el) el = prdCmtEl(item);
    const prev = tail.get(place);
    if (!keep || !el.isConnected) {
      if (slot) {
        if (prev ? prev.nextElementSibling !== el : slot.firstElementChild !== el) slot.insertBefore(el, prev ? prev.nextSibling : slot.firstChild);
      } else if ((prev || anchor).nextElementSibling !== el) (prev || anchor).after(el);
    }
    tail.set(place, el);
    placed.add(item.key);
    prdCmtControls(el, locked);
    const typed = boxes?.get(item.key);
    if (typed?.focus && ['new', 'edit'].includes(item.kind)) {
      typed.focus = false;
      el.querySelector('textarea')?.focus();
    }
  }
  have.forEach((el, key) => { if (!placed.has(key)) el.remove(); });
  prdConcernState(scope, e, doc);
}
function prdCmtEl(item) {
  const t = document.createElement('template');
  t.innerHTML = item.html;
  const el = t.content.firstElementChild;
  el.dataset.prdSig = item.sig;
  el.dataset.prdKind = item.kind;
  return el;
}
/* The comments of the PR `id` are brought up to date wherever its diff is drawn: after an edit of the drafts, a push or a hand-over, and when the
   drafts file arrives. */
function prdCommentsAll(id) {
  const cur = prdCurrent();
  if (!cur || cur.id !== id) return;
  for (const place of ['dialog', 'read']) {
    const p = prdPlace(place);
    if (p) prdComments(id, cur.e, p.root);
  }
}
/* What the send dialog needs of the diff: the layout's files, null when there is no diff to show (none stored, too big, empty, unreadable) and
   undefined while it is on its way, so that a comment is not said to be off the diff before the diff has come. */
function prdEntriesOf(id) {
  const e = prDiff.cache.get(id);
  if (!e || e.state === 'loading' || e.state === 'error' || e.key !== e.wanted) return undefined;
  return e.state === 'ok' ? e.layout.files : null;
}

/* The editors open for the PR `id`, as the page holds them. */
const prBoxesOf = id => {
  let m = prView.boxes.get(id);
  if (!m) prView.boxes.set(id, m = new Map());
  return m;
};
/* Open an editor for a new comment on the line `at` (`{ side, line }`) of `path`, or on the file with no `at`; the text starts as `text`. One
   editor per line: one that is open is given the keyboard, and filled when it is blank. Nothing is stored until 「コメントを足す」. */
function prdOpenBox(id, { path, at = null, text = '', source = 'person' }) {
  const boxes = prBoxesOf(id);
  const spec = { kind: 'new', path, side: at?.side === 'left' ? 'left' : 'right', line: at?.line ?? null, source, text, focus: true };
  const key = prdBoxKey(spec);
  const old = boxes.get(key);
  if (!old) boxes.set(key, spec);
  else {
    old.focus = true;
    if (text && !String(old.text).trim()) {
      Object.assign(old, { text, source });
      document.querySelectorAll('[data-prd-box]').forEach(ta => { if (ta.dataset.prdBox === key && ta.value !== text) ta.value = text; });
    }
  }
  prdCommentsAll(id);
}

/* A comment's box is typed in: its text is kept in the page (never stored before 「コメントを足す」 or 「保存」), and the button that needs text follows it. */
function prdInput(e) {
  const t = e.target;
  if (!t?.matches?.('[data-prd-box]')) return false;
  const id = prShown()?.id;
  const box = id ? prView.boxes.get(id)?.get(t.dataset.prdBox) : null;
  if (box) box.text = t.value;
  const el = t.closest('[data-prd-cmt]');
  if (el) prdCmtControls(el, false);
  return true;
}

/* The buttons of a comment: 編集, 削除, 保存, やめる and コメントを足す. */
function prdCommentAct(id, read, el, act) {
  const key = el.dataset.prdCmt;
  const boxes = prBoxesOf(id);
  const doc = prDoc(id, read);
  const typed = () => el.querySelector('textarea')?.value ?? boxes.get(key)?.text ?? '';
  // What cannot be edited now says why (the buttons are off while it is the case, but a key press could still get here).
  const refused = () => prEditRefused(id);
  if (act === 'edit') {
    const d = doc.drafts.find(x => x.key === key);
    if (d && !d.sentAt) boxes.set(key, { kind: 'edit', text: d.body, focus: true });
  } else if (act === 'cancel') boxes.delete(key);
  else if (act === 'delete') {
    if (!prEditDrafts(id, read, d => { d.drafts = d.drafts.filter(x => x.key !== key); })) return refused();
    boxes.delete(key);
  } else if (act === 'save') {
    const text = typed();
    if (!text.trim()) return;
    if (!prEditDrafts(id, read, d => {
      const x = d.drafts.find(y => y.key === key);
      // The line is not placed again by an edit, so the head it was written on stays.
      if (x) x.body = text;
      prStampDoc(d, read);
    })) return refused();
    boxes.delete(key);
  } else if (act === 'add') {
    const box = boxes.get(key);
    const text = typed();
    if (!box || !text.trim()) return;
    if (!prEditDrafts(id, read, d => {
      d.drafts.push(prDiffDraft(d, { path: box.path, line: box.line, side: box.side, body: text, source: box.source, sha: read?.sha }));
      prStampDoc(d, read);
    })) return refused();
    boxes.delete(key);
  } else return;
  prDraftsChanged(id);
}

/* 「コメントにする」 of a point of a read-through of the older form: an editor on the point's line of the diff (on its file with no line), filled with
   the point's words, and the diff goes there. Said, and nothing done, when the diff cannot be shown. */
function prdPointComment(point, place) {
  const cur = prdCurrent();
  const f = cur?.e.layout.files.find(x => x.path === point?.path);
  if (!cur || !f) {
    note('コメント', true, !cur ? 'この PR の差分を表示できないので、コメントを書けません（差分が保存されていないか、まだ読み込み中です）' : `差分に ${point?.path} がないので、コメントを書けません`);
    return;
  }
  // A point's line is a new number; one that is a removed line's old number goes to the left.
  const side = point.line && prDiffFindAt(f.file, 'right', point.line) == null && prDiffFindAt(f.file, 'left', point.line) != null ? 'left' : 'right';
  prdOpenBox(cur.id, { path: point.path, at: point.line ? { side, line: point.line } : null, text: `${point.title}: ${point.why}` });
  prdGoto(point.path, point.line, { place });
}

/* The click of a button of the diff. True when it was one. */
function prdAct(e, { place = 'read' } = {}) {
  const hit = sel => e.target.closest?.(sel);
  const cur = prdCurrent();
  if (!cur) return false;
  const { id, e: entry } = cur;
  const rec = prRecOf(id);
  const read = prReadOf(rec) || null;
  let b;
  if ((b = hit('[data-prd-open]'))) {
    const f = entry.layout.files[Number(b.dataset.prdOpen)];
    if (f) { prDiff.open.add(`${id}|${f.path}`); prdSyncAll(); }
    return true;
  }
  if ((b = hit('[data-prd-cmt-act]'))) {
    const el = b.closest('[data-prd-cmt]');
    if (el && !b.disabled) prdCommentAct(id, read, el, b.dataset.prdCmtAct);
    return true;
  }
  if ((b = hit('[data-prd-plus]'))) {
    const f = entry.layout.files[Number(b.closest('[data-prd-file]')?.dataset.prdFile)];
    if (f && rec) prdOpenBox(id, { path: f.path, at: prDiffAtOf(b.dataset.prdPlus) });
    return true;
  }
  if ((b = hit('[data-prd-todraft], [data-prd-ask]'))) {
    const ann = read?.annotations?.[Number(b.closest('[data-prd-ann]')?.dataset.prdAnn)];
    if (!ann || !rec) return true;
    const at = prDiffAtOf(b.dataset.prdAt);
    if (b.matches('[data-prd-todraft]')) {
      // 解説's 「コメント」 is its line's 「＋」; a concern's 「コメントにする」 brings the reader's draft (else its text) as a comment of the reader's.
      if (ann.kind === 'concern') prdOpenBox(id, { path: ann.path, at, text: String(String(ann.draft ?? '').trim() ? ann.draft : ann.text ?? ''), source: 'reader' });
      else prdOpenBox(id, { path: ann.path, at });
    } else {
      // Leaving for the terminal: the dialog goes first, and focus is not given back to the middle.
      if (place === 'dialog') { cardDialog.quiet = true; closeCardDialog(); }
      prAsk({ path: ann.path, line: at?.line, title: String(ann.text).slice(0, 40) });
    }
    return true;
  }
  return false;
}

PR_HOOKS.diff = {
  dialogFileHtml: prdDialogFileHtml, goto: prdGoto, comments: prdCommentsAll, entries: prdEntriesOf, input: prdInput, pointComment: prdPointComment,
  onMoved: prdOnMoved, onDrawn: prdOnDrawn, onDrawnAt: prdOnDrawnAt, applyTarget: prdApplyTarget, act: prdAct,
  dialogPaths: prdDialogPaths, dialogHeadHtml: prdDialogHeadHtml, dialogLabel: prdDialogLabel,
};
