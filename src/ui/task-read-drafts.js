/* ── 「worker への指示」: the drafts of a task's PR, handed to its worker ───────────────────────────────────────
   On a task's own PR a pending review does not fit (GitHub does not let you ask changes of your own PR, and the one who fixes it is
   the task's worker), so the read-through's drafts are instructions: the reader's drafts, the person's own, and a comment on the
   whole. 「N 件を worker に渡す」 sends the ones not yet sent through the task's endpoint (`POST /api/tasks/<id>/hand`), which
   answers the task's diff gate (or the latest diff record) with `changes`, as sending a gate back by hand does. What was sent is
   marked with `sentAt` and kept. The drafts themselves are the review request's (pr-panel.js, pr-read.js): this file sets the hook
   task-read-view.js leaves, `OWN_HOOKS.drafts`, and does the handing. Loaded after task-read-view.js. */

/* What the refusals of the endpoint say, over the server's own wording. */
const TASK_HAND_WHY = {
  'no-pr': 'このタスクには、まだ PR がありません',
  'pr-changed': 'タスクの PR が変わりました。下書きは前の PR のものです。開き直してください',
  'pr-done': 'この PR はマージされたか、閉じられています。worker には渡せません',
  'no-record': 'この PR は、まだ AI 下読みされていません',
  'no-worker': 'このタスクは Jules に渡してあり、worker がいません。渡せません',
  'no-gate': '差し戻せる差分の確認も記録も、このタスクにはまだありません。worker が差分を出してからにしてください',
  'gate-waiting': 'worker が別の確認（計画や質問）を待っています。先にそれに答えてください',
  busy: 'いま渡しているところです。終わるまで待ってください',
};

/* Hand the drafts of the task on show to its worker. The server reads the stored file, so the drafts are saved first (and the
   reader's, which are not a file until the first edit, are made the file). One hand-over in flight per PR: editing is locked while
   it runs, and a second click does nothing. A failed one keeps the drafts as they were and says why; a successful one marks what was
   sent with the server's stamp, clears the comment on the whole (it went with them) and saves at once, and a save that fails stays
   unsaved and is tried again like any edit. */
async function taskReadHand() {
  const task = taskReadTask();
  const shown = prShown();
  if (!task || shown?.kind !== 'task' || !shown.rec) return;
  const { id, rec } = shown;
  const e = prDraftsEntry(id);
  const read = prReadOf(rec);
  // Not before the file and the read-through are in: what is sent is what is saved.
  if (e.handing || e.pending || !e.loaded || read === undefined) return;
  const key = taskReadKey(task);
  if (!e.doc) prEditDrafts(id, read, () => {});
  e.handing = true;
  e.handResult = null;
  prRedrawNow();
  try {
    await prSaveSettled(id, '渡していません');
    const res = await boardApi(BASE, `/api/tasks/${encodeURIComponent(task.id)}/hand`, { method: 'POST', body: JSON.stringify({ pr: id }) });
    // What was sent is marked, with the stamp of the answer, which is the one `sentAt` is held to.
    const doc = structuredClone(prDoc(id, read));
    for (const d of doc.drafts) if ((res.sent || []).includes(d.key)) d.sentAt = res.at;
    if (res.overall) doc.overall = { ...(doc.overall || {}), body: '' };
    e.doc = doc; e.dirty = true; e.rev++;
    e.handResult = { sent: res.sent || [], overall: !!res.overall, present: !!res.present, woken: !!res.woken };
    note(`adj hand ${task.id}`, false, outboxNote(`${(res.sent || []).length} 件の指示を worker の outbox に追記`, res));
    prScheduleSave(id, true);
    // The answer is on the task and its gate, and on its timeline: read again, as after answering a gate.
    if (taskRead.key === key) {
      try { await Promise.all([refresh(true), taskReadRefresh(true)]); } catch {}
      refreshBoards();
    }
  } catch (err) {
    e.handResult = { error: TASK_HAND_WHY[err.why] || err.message };
    note(`adj hand ${task.id}`, true, e.handResult.error);
  } finally {
    e.handing = false;
    if (prShown()?.id === id) prRedrawNow();
  }
}

OWN_HOOKS.drafts = {
  html: (rec, read) => `<div id="tp-read-drafts" data-read-drafts>${prDraftsBodyHtml(rec, read, 'panel', 'worker')}</div>`,
  act(e) {
    const b = e.target.closest('[data-pr-hand]');
    if (!b) return false;
    if (!b.disabled) taskReadHand();
    return true;
  },
  input: e => prInput(e),
};
