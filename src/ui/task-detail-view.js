/* ── The task detail in a dialog: the page's half (the markup is task-detail.js, the stack of dialogs is dialog-stack.js) ──────────
   Opened from 「タスク詳細」 in the heading over the middle terminal in 「いまの仕事」. It reads the task from the page, draws it with
   `tocDialogHtml`, follows the page's state while it is open, marks the entry of the table of contents that is on screen, and opens
   the Issue body and the card of a question as it was asked over it as further layers. Nothing in it answers a gate: the cards the
   panel draws are taken in with their buttons removed. Loaded after work-term-view.js and question-dialog.js. */

/* What the panel's renderers put in their markup that has nothing to do in a dialog that answers nothing: the 拡大 button, the picks of
   a gate, the controls of the panel's footer. (`data-open` is not here: the timeline's are turned into openers or text below.) */
const TASK_DETAIL_DROP = '[data-expand-open], [data-pick], [data-unpick], [data-record], [data-judge], [data-tp-gate], [data-fetch-issue], [data-add-child], [data-gate], .decide, .gate-comment, [data-act], .pick[data-choice], [data-qc-ide], [data-ide], [data-focus]';

/* The detail that is open: { key, board, slug, id, task (as last drawn), head, body (as drawn), gone }. */
let taskDetail = null;
/* The entry of the table of contents the person chose, by layer key, until they scroll by hand. Any `.td-layer` (the task detail,
   a parent's detail in work-term-view.js) keeps its own. */
const tocPinned = new Map();
/* The cards of the questions that are open over a detail, by layer key: the gate as it was. */
const taskDetailCards = new Map();

/* The markup as the dialog shows it: the panel's renderers cleaned of what answers, the timeline's gate titles turned into the opener
   of the card they were, or plain text. `byId` is the task's gates by id. `frame` false leaves the frame's own ids alone (a card has
   none to keep). */
function taskDetailClean(html, byId, frame = true) {
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  const root = tpl.content;
  root.querySelectorAll(TASK_DETAIL_DROP).forEach(x => x.remove());
  root.querySelectorAll('input, textarea, select').forEach(x => { x.disabled = true; });
  // The ids the panel's renderers made belong to the panel; the frame's own (the group titles) are kept.
  root.querySelectorAll('[id], [for], [aria-labelledby], [aria-controls]').forEach(x => {
    if (frame && x.matches('.td-group, .td-group-title')) return;
    for (const a of ['id', 'for', 'aria-labelledby', 'aria-controls']) x.removeAttribute(a);
  });
  root.querySelectorAll('[data-open]').forEach(b => {
    const g = byId.get(b.dataset.open);
    if (g && g.wait !== false && g.decision) {
      b.removeAttribute('data-open');
      b.dataset.tdCard = gateRef(g);
      b.title = 'そのときのカードを開く';
    } else b.replaceWith(document.createTextNode(b.textContent));
  });
  const box = document.createElement('div');
  box.append(root);
  return box.innerHTML;
}

/* The head and the body of the detail of `task`, as the page has it now. */
function taskDetailParts(task) {
  const all = gatesOf(task);
  const isWait = g => isWaiting(g);
  const waiting = all.filter(g => g.wait !== false && isWait(g));
  const board = boards.find(b => b.slug === (task._slug || nav.board));
  const worker = workerOf(task);
  const own = sessionOfTask(task);
  const nowText = taskDetailNowText(task, {
    waiting, worker, phaseMins: worker ? phaseMinutes(worker) : null, prNote: task.pr ? prNoteOf(task) : '',
  });
  const ctx = {
    isWaiting: isWait,
    plan: gateShownIn(task, 'overview', all, {}),
    issueNumber: task.issueUrl ? issueNumberOf(task.issueUrl) : '',
    nowText,
    waiting,
    review: gateForTab(task, 'review', all, {}),
    check: gateForTab(task, 'check', all, {}),
    agent: own ? [own.agent, own.agentSession?.model].filter(Boolean).join(' · ') : '',
  };
  const frame = tocDialogHtml({
    sub: taskDetailSub(task, { repo: workRepoName(board?.nwo || ''), phase: agentLabel(agentColOf(task)) }),
    title: task.title || task.id,
    now: nowText,
    groups: taskDetailGroups(task, all, ctx),
    idPrefix: 'td',
  });
  return { head: frame.head, body: taskDetailClean(frame.body, new Map(all.map(g => [g.id, g]))) };
}

/* The `<details>` of `root` by a key that survives a redraw: the text of its own summary, and how many before it had the same. */
function taskDetailFolds(root) {
  const seen = new Map();
  return new Map([...root.querySelectorAll('details')].map(d => {
    const text = d.querySelector(':scope > summary')?.textContent || '';
    const n = seen.get(text) || 0;
    seen.set(text, n + 1);
    return [`${n}\u0000${text}`, d];
  }));
}

const taskDetailLayer = () => (taskDetail ? dialogStack.layerEl(taskDetail.key) : null);

/* The entry of the table of contents that is current: the one for `key`. */
function tocMark(layer, key) {
  layer.querySelectorAll('[data-td-go]').forEach(b => {
    if (b.dataset.tdGo === key) b.setAttribute('aria-current', 'location'); else b.removeAttribute('aria-current');
  });
}

/* The entry that is current by where the body is scrolled to. */
function tocSpy(layer) {
  const body = layer.querySelector('.qd-body');
  const groups = [...body.querySelectorAll('[data-td-group]')];
  if (!groups.length) return;
  const top = body.getBoundingClientRect().top;
  const tops = groups.map(g => g.getBoundingClientRect().top - top + body.scrollTop);
  tocMark(layer, groups[tocActiveKey(tops, body.scrollTop, body.clientHeight, body.scrollHeight)].dataset.tdGroup);
}

/* Marks the entry that is on screen as the body of a `.td-layer` scrolls, until the person chooses one (the click handler below) and
   scrolls by hand again. Called once, when the layer is pushed. */
function tocWire(layer) {
  const key = layer.dataset.qdKey;
  const scroller = layer.querySelector('.qd-body');
  // The person scrolling by hand takes the choice back from the entry they clicked.
  const release = () => { tocPinned.delete(key); };
  scroller.addEventListener('wheel', release, { passive: true });
  scroller.addEventListener('touchmove', release, { passive: true });
  // A drag on the scrollbar has the body itself as its target.
  scroller.addEventListener('pointerdown', e => { if (e.target === scroller) release(); });
  scroller.addEventListener('keydown', e => { if (e.key !== 'Tab') release(); });
  let queued = false;
  scroller.addEventListener('scroll', () => {
    if (queued || tocPinned.has(key)) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      if (!tocPinned.has(key) && layer.isConnected && !layer.hidden) tocSpy(layer);
    });
  }, { passive: true });
}

/* A `.td-layer` drawn again with `parts` ({ head, body }) as its dialog's `info` ({ key, head, body }) had them: keeping where it was
   scrolled to, which entry is current and which sections were open, and not while the focus is in something of the body other than the
   body itself or the table of contents. */
function tocRedraw(layer, info, parts) {
  if (parts.head === info.head && parts.body === info.body) return;
  const body = layer.querySelector('.qd-body');
  const active = document.activeElement;
  const inside = active !== body && body.contains(active);
  const calm = inside && active.matches('[data-td-go], [data-td-group]');
  if (inside && !calm) return;
  const top = body.scrollTop;
  const current = layer.querySelector('[data-td-go][aria-current]')?.dataset.tdGo;
  const focusKey = calm ? (active.dataset.tdGo ?? active.dataset.tdGroup) : null;
  const focusWas = calm ? (active.matches('[data-td-go]') ? 'go' : 'group') : '';
  // The heading names the dialog (dialog-stack.js sets its id): a new one takes the id of the old.
  const main = layer.querySelector('.qd-head-main');
  const titleId = main.querySelector('h2')?.id;
  main.innerHTML = parts.head;
  if (titleId) main.querySelector('h2').id = titleId;
  // What was unfolded or folded by hand stays so: each fold is known by its summary and which of that summary it is, so one that
  // comes or goes does not move the others' state (only one with the very same summary, before it, would).
  const folds = taskDetailFolds(body);
  body.innerHTML = parts.body;
  for (const [key, d] of taskDetailFolds(body)) if (folds.has(key)) d.open = folds.get(key).open;
  body.scrollTop = top;
  if (current) tocMark(layer, current);
  // What was drawn can be taller or shorter now: the entry follows unless the person chose one.
  if (!tocPinned.has(info.key)) tocSpy(layer);
  if (focusKey != null) {
    body.querySelector(focusWas === 'go' ? `[data-td-go="${CSS.escape(focusKey)}"]` : `[data-td-group="${CSS.escape(focusKey)}"]`)?.focus({ preventScroll: true });
  }
  info.head = parts.head;
  info.body = parts.body;
}

function openTaskDetail(task, opener) {
  const key = `detail:${nav.board}/${task.id}`;
  if (dialogStack.has(key)) return;
  if (dialogStack.isOpen()) dialogStack.closeAll();
  const { head, body } = taskDetailParts(task);
  taskDetail = { key, board: nav.board, slug: task._slug || '', id: task.id, task, head, body, gone: false };
  dialogStack.push({
    key, cls: 'td-layer', head, body, opener,
    onClose: layer => { tocPinned.delete(layer.key); if (taskDetail?.key === layer.key) taskDetail = null; },
  });
  tocWire(dialogStack.layerEl(key));
}

/* The open detail as the page's state now has it. It is drawn again only when its markup changed, keeping where it was scrolled to
   and which entry is current, and not while the focus is in something of the body other than the body itself or the table of
   contents. A task the state no longer lists is marked once and left as it was. A detail of another board than the page's is left. */
function taskDetailFollow() {
  const info = taskDetail;
  if (!info || !dialogStack.isOpen() || info.gone || nav.board !== info.board) return;
  const layer = taskDetailLayer();
  if (!layer) return;
  const task = (state.tasks || []).find(t => t.id === info.id && (!info.slug || t._slug === info.slug));
  if (!task) {
    // A state not read yet lists nothing: not the task being gone.
    if (state.now == null) return;
    info.gone = true;
    layer.querySelector('.qd-head')?.insertAdjacentHTML('afterend', '<p class="qd-gone" role="status">このタスクは、いまの一覧にありません（最後に表示した内容です）</p>');
    return;
  }
  info.task = task;
  // Under another layer (a card, the Issue) the detail has no box: a redraw there would lose where it was scrolled to and mark the
  // wrong entry. It is drawn when it is on top again, at the next render.
  if (layer.hidden) return;
  tocRedraw(layer, info, taskDetailParts(task));
}

/* ── the table of contents and the layers over the detail ── */

/* The card of a question as it was asked, read only, over the detail. */
function openTaskDetailCard(info, ref, opener) {
  const g = gatesOf(info.task).find(x => gateRef(x) === ref);
  if (!g) return note('開く', true, 'この質問の記録は読めません');
  const key = `was:${ref}`;
  if (dialogStack.has(key)) return;
  const c = questionCardHtml(g, { ...questionCtxOf(g), key: gateKey(g), readOnly: true });
  const how = `<p class="tp-muted td-was">${esc(`${when(g.answeredAt)} に ${taskDetailPickText(g)}`)}</p>`;
  taskDetailCards.set(key, { g, task: info.task });
  dialogStack.push({
    key, head: c.head, body: taskDetailClean(c.body + how, new Map(), false), opener,
    // The detail under it was not drawn while it was covered: it is, once it is on top again.
    onClose: layer => { taskDetailCards.delete(layer.key); setTimeout(taskDetailFollow); },
  });
}

questionDialogEl().addEventListener('click', e => {
  const go = e.target.closest?.('[data-td-go]');
  const info = taskDetail;
  if (go && go.closest('.td-layer')) {
    const layer = go.closest('.td-layer');
    const body = layer.querySelector('.qd-body');
    const group = [...body.querySelectorAll('[data-td-group]')].find(x => x.dataset.tdGroup === go.dataset.tdGo);
    if (!group) return;
    const calm = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    tocPinned.set(layer.dataset.qdKey, go.dataset.tdGo);
    tocMark(layer, go.dataset.tdGo);
    group.scrollIntoView({ block: 'start', behavior: calm ? 'auto' : 'smooth' });
    group.focus({ preventScroll: true });
    return;
  }
  const issue = e.target.closest?.('[data-td-issue]');
  const card = e.target.closest?.('[data-td-card]');
  if (!(issue || card) || !info || !(issue || card).closest('.td-layer')) return;
  if (info.board !== nav.board) return note('タスク詳細', true, '別のボードに移ったため、ここからは開けません');
  if (issue) {
    const layer = issueLayerOf(info.task, info.task.issueUrl ? issueNumberOf(info.task.issueUrl) : '');
    if (!layer) return note('開く', true, 'Issue の本文は保存されていません');
    dialogStack.push({ key: `td-issue:${info.key}`, opener: issue, onClose: () => setTimeout(taskDetailFollow), ...layer });
    return;
  }
  openTaskDetailCard(info, card.dataset.tdCard, card);
}, true);

/* The buttons of a card as it was asked that open a layer need the gate the card was drawn from, which a read-only card does not carry
   (it answers nothing), so they are answered here, before `question-dialog.js` looks for a root they do not have. */
questionDialogEl().addEventListener('click', e => {
  const open = e.target.closest?.('[data-qc-open]');
  const layer = open?.closest('.qd-layer');
  const was = layer && taskDetailCards.get(layer.dataset.qdKey);
  if (!was) return;
  e.stopPropagation();
  const { g, task } = was;
  const title = task?.title || g.title || '';
  if (open.dataset.qcOpen === 'issue') {
    const issue = issueLayerOf(task, task?.issueUrl ? issueNumberOf(task.issueUrl) : '');
    if (!issue) return note('開く', true, 'Issue の本文は保存されていません');
    return void dialogStack.push({ key: `issue:${layer.dataset.qdKey}`, opener: open, ...issue });
  }
  if (open.dataset.qcOpen === 'html-report') return void openGateReport(g, open, title);
  if (open.dataset.qcOpen === 'report') {
    return void dialogStack.push({ key: `report:${layer.dataset.qdKey}`, opener: open, title: `レポート — ${title}`, body: `<div class="body">${md(g.body)}</div>` });
  }
}, true);

registerView('task-detail', { render: () => taskDetailFollow() });
