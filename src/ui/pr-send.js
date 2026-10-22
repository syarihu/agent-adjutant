/* ── 「レビューを送る」 and 「worker に渡す」: the dialog that sends the comments ───────────────────────────────────────────
   The comments are written on the lines of the diff (pr-diff-view.js). Sending them is one dialog (`#review-send-dialog`, a modal one, so that it
   also stands over 「拡大」): for a review request 「レビューを送る」, which puts them into the person's pending review on GitHub (`prPend`), for a
   task's own read-through 「worker に渡す」, which hands them to the task's worker (`taskReadHand`). Both have the overall comment in it, which is
   the reader's own text until the person types in it. The comments the diff cannot show (no file, or a file the diff does not have) are listed here,
   where they can be deleted. What the dialog says is pr-read.js's (pure); this file draws it, and keeps it in step with the drafts. Loaded after
   task-read-drafts.js. */

const prSend = {
  id: null,      // the PR the dialog is open for, null while it is closed
  to: 'pending', // 'pending' (a review request's) or 'worker' (a task's)
  drawn: false,  // the overall comment's box is drawn: it is drawn once per opening, so that what is typed in it is never drawn over
};
const prSendEl = () => document.getElementById('review-send-dialog');

/* Open the dialog for what is on show: a review request's pending review, or a task's worker. */
function prSendOpen() {
  const dlg = prSendEl();
  const shown = prShown();
  if (!dlg || dlg.open || !shown?.rec) return;
  const to = shown.kind === 'task' ? 'worker' : 'pending';
  Object.assign(prSend, { id: shown.id, to, drawn: false });
  dlg.querySelector('#review-send-title').textContent = to === 'worker' ? 'worker に渡す' : 'レビューを送る';
  // What an earlier opening drew is not this one's: the overall comment's box is drawn anew.
  for (const part of ['text', 'rest', 'foot']) {
    const el = dlg.querySelector(`#review-send-${part}`);
    el.replaceChildren();
    delete el.dataset.html;
  }
  prSendSync(shown.id, true);
  dlg.returnValue = '';
  dlg.showModal();
  dlg.querySelector('.pr-overall')?.focus({ preventScroll: true });
}

/* Bring the dialog up to date with the drafts of `id` (after an edit, a push, a hand-over, or the drafts arriving), if it is open for that PR. The overall
   comment's box is left as it is while it is being typed in; the rest is drawn when its markup changes. `opening` is the first draw, before the
   dialog is open. */
function prSendSync(id, opening = false) {
  const dlg = prSendEl();
  if (!dlg || !(dlg.open || opening) || prSend.id !== id) return;
  const shown = prShown();
  // The PR is not the one on show any more (or is gone from the list): nothing here is about it.
  if (shown?.id !== id || !shown.rec) { if (dlg.open) dlg.close(); return; }
  const { rec } = shown;
  const read = prReadOf(rec);
  const entry = prDraftsEntry(id);
  const text = dlg.querySelector('#review-send-text');
  const rest = dlg.querySelector('#review-send-rest');
  const foot = dlg.querySelector('#review-send-foot');
  const put = (el, html) => { if (el.dataset.html !== html) { el.innerHTML = html; el.dataset.html = html; } };
  // Neither the read-through nor the drafts are here yet: what is sent is what was saved.
  if (read === undefined || !entry.loaded) {
    prSend.drawn = false;
    put(text, '');
    put(rest, `<div class="tp-muted" role="status">${esc(entry.error ? `保存済みのコメントを読めませんでした（読み直しています）: ${entry.error}` : '読み込み中…')}</div>`);
    put(foot, '<button type="submit" value="cancel" class="btn-m3-text">閉じる</button>');
    return;
  }
  const worker = prSend.to === 'worker';
  const doc = prDoc(id, read || null);
  if (!prSend.drawn) {
    prSend.drawn = true;
    put(text, prSendTextHtml(doc, read, prSend.to));
  }
  const shownText = prOverallShown(doc, read);
  const box = text.querySelector('.pr-overall');
  const sending = worker ? entry.handing : entry.pending;
  if (box) {
    box.readOnly = sending;
    // What a hand-over cleared, or a push stored, is what the box says; one being typed in already says it.
    if (document.activeElement !== box && box.value !== shownText.text) box.value = shownText.text;
  }
  const chip = text.querySelector('[data-pr-send-chip]');
  if (chip) chip.hidden = !shownText.reader;
  // With no diff to show, every comment with a file is off it; while the diff is on its way none is known to be.
  const entries = read?.mode === 'split' || !PR_HOOKS.diff ? null : PR_HOOKS.diff.entries(id);
  const un = prDiffUnplaced(entries, doc.drafts);
  const o = { to: prSend.to, sending, result: worker ? entry.handResult : entry.pendResult, prUrl: rec.url, off: un.off, whole: un.whole };
  put(rest, prSendRestHtml(doc, read, o));
  put(foot, prSendFootHtml(doc, read, o));
  prShowSave(id);
}

const prSendDialog = prSendEl();
prSendDialog.addEventListener('click', e => {
  const shown = prShown();
  if (!prSend.id || shown?.id !== prSend.id || !shown.rec) return;
  const id = prSend.id;
  const read = prReadOf(shown.rec) || null;
  let b;
  if ((b = e.target.closest('[data-pr-pend]'))) {
    if (!b.disabled) prPend(id, read);
  } else if ((b = e.target.closest('[data-pr-hand]'))) {
    if (!b.disabled) taskReadHand();
  } else if (e.target.closest('[data-pr-copy-all]')) prCopy(prCopyAllText(prDocShown(prDoc(id, read), read)));
  else if ((b = e.target.closest('[data-pr-discard]'))) {
    const key = b.dataset.prDiscard;
    if (!prEditDrafts(id, read, d => { d.drafts = d.drafts.filter(x => x.key !== key); })) prEditRefused(id);
    else prDraftsChanged(id);
  }
});
// Typing in the overall comment: kept in the document and saved a moment after; what the buttons say follows it.
prSendDialog.addEventListener('input', e => {
  if (!e.target.matches?.(PR_BOX_SEL)) return;
  prInput(e);
  prSendSync(prSend.id);
});
prSendDialog.addEventListener('focusout', e => prFocusOut(e));
// Closing saves at once, as closing 「拡大」 does, and the heading's count and the lines are drawn from what was sent.
prSendDialog.addEventListener('close', () => {
  const id = prSend.id;
  prSend.id = null;
  prSend.drawn = false;
  if (!id) return;
  prScheduleSave(id, true);
  prDraftsChanged(id);
});
