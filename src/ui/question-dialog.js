/* ── The question card in a dialog: the page's half (the markup is question-card.js, the stack of dialogs is dialog-stack.js) ──────
   Opens a waiting gate as a card, keeps it in step with the page's state while it is open, opens the full diff, the Issue body and
   the report over it as further layers, and answers through `answerGate` and `closeGateOf`, not through
   `bindDecide`, whose comment box is looked up in the task panel and the 「拡大」 dialog. The card is also answered where it stands: a quick
   question over the terminal in 「いまの仕事」 (it slides down over the terminal's top: work-term-view.js), and a heavy one is opened
   from there. The task panel's tabs and the 「拡大」 dialog keep their own answer area until the card replaces it there. Loaded after
   dialog-stack.js and before my-work.js. */

/* What was typed in a card's comment box and not sent, by `gateKey`; kept while the page is. */
const questionDrafts = new Map();

/* The card that is the first layer of the dialog, or null: { key, g (the gate as it was opened, pinned to its board), ctx, body (as
   drawn), gone, opts }. */
let questionBase = null;

/* The gate with the board it came from named, so that an answer is sent there whichever board the page is on by then. A page that
   serves several boards tags the gates of a merged state; one board's own state does not, and the board is the page's. */
function questionPin(g) {
  return g._slug || !multiBoard || !nav.board || nav.board === 'all' ? g : { ...g, _slug: nav.board, _base: `/b/${nav.board}` };
}

/* Where a gate of another board than the page's can be found: `fn(key)` answers `{ g, data }` (the gate, pinned to its board, and the
   state of that board it was read from, and `at` when, in ms), `{ gone: true }` (the board was read and no longer lists it as waiting) or null (not known). The TODO list registers its cache of the boards it has read (todo-view.js). */
const questionSources = [];
function registerQuestionSource(fn) { questionSources.push(fn); }
function questionSourceOf(key) {
  for (const fn of questionSources) {
    const found = fn(key);
    if (found) return found;
  }
  return null;
}

/* `src` (what a source answered) when it holds a gate and was read within `TODO_BOARD_FRESH_MS`, else null. */
const questionSourceFresh = src => (src && !src.gone && src.data && Date.now() - src.at < TODO_BOARD_FRESH_MS ? src : null);

/* The gate the page lists under `key` now, or null: not one answered in this page, and from the state of the board it is on, else
   from what a source has read of its own board. */
function questionLive(key) {
  if (answeredGates.has(key)) return null;
  const src = questionSourceOf(key);
  // The TODO view does not refresh the page's state, so there a source's read of the board is the fresher, and a board that was read
  // and does not list the gate (`gone`) says it is not waiting, whatever the state remembers.
  if (view === 'todo' && src) return src.gone ? null : src.g;
  // Elsewhere the page's state comes first, and a source answers only while its read of the board is young: it is read only while
  // the TODO or its popover is on, so an old one may hold a gate that has been answered since.
  return (state.gates || []).find(x => gateKey(x) === key) || questionSourceFresh(src)?.g || null;
}

/* `live` (as the page lists it) with the board `like` was opened on, so a card follows the gate and keeps sending to its board. */
const pinLike = (live, like) => (like?._slug && !live._slug ? { ...live, _slug: like._slug, _base: like._base } : live);

/* ── The AI 下読み of a diff gate's PR ── */

/* The read-through of the PR of a task, by `<board>|<task id>|<PR>`: { at (when it was last asked for), busy, read (null for none), error }.
   It is asked for only for the card that is open (a list of cards, such as the TODO, reads what is held and asks for nothing), and again
   after this long. At most `QUESTION_READS_KEEP` are kept, the one used longest ago going first. */
const questionReads = new Map();
const QUESTION_READ_MS = 60 * 1000;
const QUESTION_READS_KEEP = 8;

const questionReadKey = (g, task) => `${g._slug || nav.board || ''}|${task.id}|${task.pr}`;
/* What names a read-through, to tell a changed one from the same one again. */
const questionReadSig = read => (read ? `${read.sha}|${read.madeAt}` : '');

/* The read-through that backs the diff of `g`'s task, as held, or null: the gate is not a diff gate, the task has no PR, the read-through
   was not of the PR's head when it was last asked for, or it has not been read (`questionReadEnsure` asks for it). What is held is
   given until the next answer replaces it. */
function questionReadOf(g, task) {
  if (g.kind !== 'diff' || !task?.pr) return null;
  const key = questionReadKey(g, task);
  const e = questionReads.get(key);
  if (!e) return null;
  // Used now: it is the last to go.
  questionReads.delete(key);
  questionReads.set(key, e);
  return e.read;
}

/* Ask for the read-through of the open card's task when it is not held or was read a while ago; the card is drawn again when it comes. */
function questionReadEnsure(g, task) {
  if (g.kind !== 'diff' || !task?.pr) return;
  const key = questionReadKey(g, task);
  const e = questionReads.get(key);
  if (!e || (!e.busy && Date.now() - e.at > QUESTION_READ_MS)) questionReadFetch(g, task, key);
}

/* Ask for the read-through of the task's PR: the task's document says which record and whether the read-through is of its head now, then
   the stored read-through is read and kept only if it is that one, has annotations and is not a split proposal. No document, record,
   read-through or stored copy is an answer (none); any other failure keeps the read-through held and is asked again after a while. The
   answer goes to the entry it was asked for, and the open card is drawn again only if it is of that task. */
async function questionReadFetch(g, task, key) {
  let e = questionReads.get(key);
  if (!e) {
    e = { at: 0, busy: false, read: null, error: null };
    // Past the limit the one used longest ago goes, but not one whose answer is still on its way.
    for (const [k, x] of [...questionReads]) {
      if (questionReads.size < QUESTION_READS_KEEP) break;
      if (!x.busy) questionReads.delete(k);
    }
  } else questionReads.delete(key);
  questionReads.set(key, e);
  e.at = Date.now();
  e.busy = true;
  const was = questionReadSig(e.read);
  let rec = null;
  try {
    const doc = await boardApi(g._base || BASE, `/api/tasks/${encodeURIComponent(task.id)}/ai`);
    rec = doc?.record;
    let read = null;
    if (rec?.ai?.sha && !rec.ai.stale) {
      const got = await boardApi('', `/api/others/read?id=${encodeURIComponent(rec.id)}`);
      if (got && got.sha === rec.ai.sha && got.mode !== 'split' && got.annotations?.length) read = got;
    }
    e.read = read;
    e.error = null;
  } catch (err) {
    if (err.status === 404) { e.read = null; e.error = null; } else {
      e.error = err.message;
      // What is held is kept only while it is still of the head the record names: a newer read-through that could not be read is not the old one.
      if (rec && e.read && (!rec.ai?.sha || rec.ai.stale || e.read.sha !== rec.ai.sha)) e.read = null;
    }
  } finally {
    e.busy = false;
  }
  if (questionReadSig(e.read) !== was) questionReadRedraw(key);
}

/* What else draws a card from `questionReads` and is told when one came or changed (the TODO's selected card). */
const questionReadListeners = [];

/* A read-through came or changed: draw the open card again if it is the card of a gate whose task it is for, and tell the others. */
function questionReadRedraw(key) {
  questionReadListeners.forEach(fn => fn(key));
  const base = questionBase;
  const task = base?.ctx?.task;
  if (!base || base.gone || !task?.pr || !dialogStack.isOpen() || questionReadKey(base.g, task) !== key) return;
  const layer = dialogStack.layerEl(`card:${base.key}`);
  if (!layer) return;
  const read = questionReads.get(key)?.read ?? null;
  const ctx = { ...base.ctx, read };
  questionBodyReplace(layer, base, questionCardHtml(base.g, ctx), base.g, ctx);
}

/* What the card is given besides the gate: its task, the plan it carries out, where it comes from. */
function questionCtxOf(g) {
  const task = taskForGate(g) || null;
  const board = g._slug ? boards.find(b => b.slug === g._slug) : null;
  const snap = task?.issueSnapshot;
  const url = task?.issueUrl || snap?.url || '';
  return {
    task,
    plan: g.kind === 'plan' ? g : task?.approvedPlan || null,
    // The AI 下読み of the task's PR (a diff gate only): null until it has been read, and when there is none that backs the diff.
    read: questionReadOf(g, task),
    nwo: board?.nwo || '',
    key: gateKey(g),
    issue: { number: url ? issueNumberOf(url) : '', url, hasBody: !!snap?.body },
    now: Date.now() / 1000,
    ticked: manualChecked,
  };
}

/* `questionCtxOf`, read from the board's own state when the gate is one a source holds and the page's state does not. */
function questionCtxFor(g) {
  const key = gateKey(g);
  const found = questionSourceOf(key);
  const src = view === 'todo' ? found : (state.gates || []).some(x => gateKey(x) === key) ? null : questionSourceFresh(found);
  return src?.data ? withViewState(src.data, () => questionCtxOf(g)) : questionCtxOf(g);
}

/* Opens the card of a waiting gate. `opts`: `ctx` (what `questionCtxOf` makes, from another board's records), `opener` (the element
   focus returns to), `onAnswered(g)`. A gate that no longer waits has no card. */
function openQuestionDialog(g, opts = {}) {
  if (g.wait === false) return;
  const ctx = opts.ctx || questionCtxFor(g);
  const pinned = questionPin(g);
  const key = gateKey(pinned);
  if (dialogStack.has(`card:${key}`)) return;
  // Another card is on: it is left, as the board's dialogs are, before this one is.
  if (dialogStack.isOpen()) dialogStack.closeAll();
  const c = questionCardHtml(pinned, { ...ctx, key, draft: questionDrafts.get(key) || '' });
  questionBase = { key, g: pinned, ctx: { ...ctx, key }, body: c.body, gone: false, opts };
  questionReadEnsure(pinned, ctx.task);
  dialogStack.push({
    key: `card:${key}`, head: c.head, body: c.body, foot: c.foot, gate: key, opener: opts.opener,
    onClose: layer => {
      // What was typed stays for the next opening; a sent answer has cleared it.
      const typed = dialogStack.layerEl(layer.key)?.querySelector('.gate-comment')?.value;
      if (typed != null) typed ? questionDrafts.set(key, typed) : questionDrafts.delete(key);
      if (questionBase?.key === key) questionBase = null;
    },
  });
}

/* ── The layers over the card ── */

/* The diff of a gate, a block to a file. Not cut short, as `prDiffExcerptHtml` is: this layer is where it is read whole. */
function questionDiffBodyHtml(diff) {
  const chunks = String(diff || '').split(/^(?=diff --git )/m).filter(x => x.trim());
  return chunks.map(chunk => {
    const path = prParseDiff(chunk)[0]?.path;
    return `${path ? `<div class="qc-file"><code>${esc(path)}</code></div>` : ''}<div class="diff">${renderDiff(chunk.replace(/\n$/, ''))}</div>`;
  }).join('');
}

/* The title and the body of the layer that reads a task's Issue, or null when no body is kept. `number` is the Issue's number, or ''. */
function issueLayerOf(task, number) {
  const snap = task?.issueSnapshot;
  if (!snap?.body) return null;
  const url = httpUrl(snap.url || task.issueUrl);
  return {
    title: `Issue${number ? ` #${number}` : ''} ${snap.title || ''}`.trim(),
    body: `<div class="body">${md(snap.body)}</div>`
      + (snap.truncated ? '<p class="qc-note">先頭のみ保存</p>' : '')
      + (url ? `<a class="qc-link" href="${esc(url)}" target="_blank" rel="noopener noreferrer"><span class="material-symbols-outlined" aria-hidden="true">open_in_new</span><span>GitHub で開く</span></a>` : ''),
  };
}

/* `what` is the `data-qc-open` of the button: diff, issue, report (the gate's markdown body) or html-report (the HTML file it carries). */
function questionOpenLayer(what, root) {
  const key = root.closest('[data-qc-gate]')?.dataset.qcGate;
  const g = key && (questionBase?.key === key ? questionBase.g : questionLive(key));
  if (!g) return note('開く', true, 'この質問は、いま読めません');
  const ctx = questionBase?.key === key ? questionBase.ctx : questionCtxFor(g);
  const title = ctx.task?.title || g.title || '';
  const layer = { key: `${what}:${key}`, opener: root };
  if (what === 'diff') {
    const files = qcDiffFiles(g.diff);
    const add = files.reduce((n, f) => n + f.add, 0);
    const del = files.reduce((n, f) => n + f.del, 0);
    return dialogStack.push({ ...layer, title: `差分 — ${title}（${files.length} ファイル · +${add} −${del}）`, body: questionDiffBodyHtml(g.diff) || '<p class="tp-muted">差分はありません</p>' });
  }
  if (what === 'issue') {
    const issue = issueLayerOf(ctx.task, ctx.issue.number);
    if (!issue) return note('開く', true, 'Issue の本文は保存されていません');
    return dialogStack.push({ ...layer, ...issue });
  }
  if (what === 'html-report') return openGateReport(g, root, title);
  if (what === 'report') return dialogStack.push({ ...layer, title: `レポート — ${title}`, body: `<div class="body">${md(g.body)}</div>` });
}

/* ── Answering ── */

/* A click on a button of the card or of a quick row. The gate is read fresh, for what was sent must be what is still asked; the
   one the card was opened with stands in when the page is on another board. True when the answer went. */
async function questionAct(b) {
  const root = b.closest('[data-qc-gate]');
  if (!root) return false;
  const key = root.dataset.qcGate;
  const live = questionLive(key);
  const snapshot = questionBase?.key === key && !questionBase.gone ? questionBase.g : null;
  const g = live ? pinLike(questionPin(live), snapshot) : snapshot && !answeredGates.has(key) ? snapshot : null;
  if (!g) return note('判断', true, 'この質問は、もう待っていません');
  if (b.matches('[data-qc-ide]')) return worktreeAct('ide', g.worktree, false, g._slug || null);
  const act = b.matches('.pick[data-choice]') ? 'choice' : b.dataset.act;
  if (act === 'talk') { dialogStack.closeAll(); talk(gateRef(g)); return false; }
  if (!['choice', 'close', ...(g.options || [])].includes(act)) return false;
  // A send-back with nothing written says nothing to the worker: ask for the comment before anything is locked or sent.
  // A quick row has no box to write in, and is answered as it stands.
  const box = root.querySelector('.gate-comment');
  if (qcNeedsComment(act, g) && box && !box.value.trim()) {
    note('判断', true, act === 'ask' ? '聞きたいことをコメントに書いてください' : act === 'answer' ? '答えをコメントに書いてください' : '直してほしいところをコメントに書いてください');
    box.focus();
    return false;
  }
  const ref = gateRef(g);
  if (deciding.has(ref)) return false;
  deciding.add(ref);
  const buttons = [...root.querySelectorAll('button')].filter(x => !x.disabled);
  buttons.forEach(x => { x.disabled = true; });
  const typed = root.querySelector('.gate-comment')?.value.trim() || '';
  let sent = false;
  try {
    sent = act === 'close' ? await closeGateOf(g, typed) : await answerGate(g, act, act === 'choice' ? b.dataset.choice : undefined, typed);
  } finally {
    deciding.delete(ref);
    // A refused answer keeps what was typed, and the buttons.
    if (!sent) buttons.forEach(x => { if (x.isConnected) x.disabled = false; });
  }
  if (!sent) return false;
  const onAnswered = questionBase?.key === key ? questionBase.opts.onAnswered : null;
  questionDrafts.delete(key);
  if (questionBase?.key === key) dialogStack.closeAll();
  dropGate(questionLive(key) || g);
  await refresh(true);
  refreshBoards();
  onAnswered?.(g);
  return true;
}

const questionDialogEl = () => document.getElementById('question-dialog');
questionDialogEl().addEventListener('click', e => {
  const open = e.target.closest('[data-qc-open]');
  if (open) return questionOpenLayer(open.dataset.qcOpen, open);
  const b = e.target.closest('.pick[data-choice], [data-act], [data-qc-ide]');
  if (b && !b.disabled) questionAct(b);
});
questionDialogEl().addEventListener('input', e => {
  const box = e.target.closest?.('.gate-comment');
  const key = box?.closest('[data-qc-gate]')?.dataset.qcGate;
  if (!key) return;
  if (box.value) questionDrafts.set(key, box.value); else questionDrafts.delete(key);
});

/* ── Following the page ── */

/* Put the markup `c.body` of the card in the part that scrolls, if it differs from what is drawn and the focus is not in something of
   it; the card then holds `g` and `ctx`. The scroll stays where it was. True when it was replaced. */
function questionBodyReplace(layer, base, c, g, ctx) {
  const body = layer.querySelector('.qd-body');
  if (!body || c.body === base.body || (document.activeElement !== body && body.contains(document.activeElement))) return false;
  const top = body.scrollTop;
  body.innerHTML = c.body;
  body.scrollTop = top;
  base.body = c.body;
  base.g = g;
  base.ctx = ctx;
  return true;
}

/* The open card as the page's state now has it. The part that scrolls is redrawn only when its markup changed, keeping where it was
   scrolled to, and not while the focus is in something of it; the foot is never redrawn, so what is typed in it is never lost. A gate the board no
   longer lists is shown as gone: nothing to send, the way back is the head's. A card of another board than the page's is left as it is. */
function questionDialogFollow() {
  const base = questionBase;
  if (!base || !dialogStack.isOpen() || base.gone) return;
  questionReadEnsure(base.g, base.ctx?.task);
  if (multiBoard && base.g._slug !== nav.board) return;
  const layer = dialogStack.layerEl(`card:${base.key}`);
  if (!layer) return;
  const live = questionLive(base.key);
  // A state not read yet lists nothing: not the gate being gone.
  if (!live && state.now == null) return;
  if (live) {
    const pinned = pinLike(live, base.g);
    const ctx = { ...questionCtxOf(live), key: base.key };
    questionBodyReplace(layer, base, questionCardHtml(pinned, ctx), pinned, ctx);
    return;
  }
  base.gone = true;
  layer.querySelectorAll('.qd-foot .decide button, .qd-body .pick').forEach(x => { x.disabled = true; });
  const box = layer.querySelector('.qd-foot .gate-comment');
  if (box) box.readOnly = true;
  if (!layer.querySelector('.qd-gone')) layer.querySelector('.qd-head').insertAdjacentHTML('afterend', '<p class="qd-gone" role="status">この質問は、もう待っていません（答えたか、閉じられました）</p>');
}

registerView('question-dialog', { render: () => questionDialogFollow() });
