/* ── 「拡大」 for a PR waiting on your review ─────────────────────────────────────────────────────────────────
   The dialog of a whole task (card-dialog.js) with a PR in it: the index on the left, the read-through in one scroll, and the
   drafts in the column on the right, where a task's gate answers sit. Its machinery is the task dialog's: `cardDialogTaskHtml`
   draws the index and the content from groups, `cardDialogChanged` marks what changed while it is open. The groups are
   pr-read.js's (`prDialogGroups`); this file draws them, keeps the column, and follows the panel. Loaded after pr-panel.js. */

const PR_DIALOG_GONE = 'パネルからこの PR がなくなりました（最後に表示した内容です）';
const PR_DIALOG_NO_READ = 'AI 下読みすると下書きの候補が入ります。';
const PR_DIALOG_NO_SPLIT = '分割を提案すると、下書きに分割案が入ります。';

/* A control of the column, found again after the column is drawn anew: by the first `data-pr-*` attribute it has. */
function prFocusSel(el) {
  const key = Object.keys(el?.dataset || {}).find(k => k.startsWith('pr'));
  if (!key) return null;
  const attr = key.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`);
  return `[data-${attr}="${CSS.escape(el.dataset[key])}"]`;
}

/* The diff's cards are not marked as changed: they come and go with the read-through and with the diff's own loading, and what
   they say is the diff itself, not news about the PR. The other cards keep their marks. */
function prDialogWithoutDiff(changes, groups) {
  const own = k => k === 'diff-bar' || k.startsWith('diff:');
  changes.cards = changes.cards.filter(k => !own(k));
  changes.groups = new Set(groups.filter(g => g.cards.some(c => changes.cards.includes(c.key))).map(g => g.key));
  changes.firstKey = changes.cards[0] ?? null;
  return changes;
}

/* What the content was last drawn from: the cards in order, and each card's markup. A draw that has the same cards in the same
   places touches only the cards whose markup changed (`prDialogPatch`). */
const prDialogDrawn = { order: '', html: new Map() };

/* Put `frag` (the content as nodes) into `content`. A card is the person's to select and scroll in, so when only some cards
   changed (a relative time in 要点 changes every minute) only those are replaced, and the others, with the diff files drawn in
   them, stay as they are. Different cards or places, or a first draw, replace the whole content. `drawn` is what `content` was
   last drawn from: the dialog's, or the one a task's read-through keeps for itself. */
function prDialogPatch(content, frag, groups, drawn = prDialogDrawn) {
  const order = groups.map(g => `${g.key}[${g.cards.map(c => c.key).join('|')}]`).join('\n');
  const html = new Map(groups.flatMap(g => g.cards.map(c => [c.key, c.html])));
  const was = drawn.html;
  const kept = content.children;
  const same = drawn.order === order && !!kept && kept.length === groups.length && !!frag.children && frag.children.length === groups.length;
  drawn.order = order;
  drawn.html = html;
  if (!same) return content.replaceChildren(frag);
  groups.forEach((g, i) => {
    const old = kept[i];
    const fresh = frag.children[i];
    const title = old.querySelector('.cd-group-title');
    const newTitle = fresh.querySelector('.cd-group-title');
    if (title && newTitle && title.innerHTML !== newTitle.innerHTML) title.replaceWith(newTitle);
    const oldCards = old.querySelectorAll('[data-cd-card]');
    const newCards = fresh.querySelectorAll('[data-cd-card]');
    g.cards.forEach((c, j) => { if (was.get(c.key) !== c.html) oldCards[j].replaceWith(newCards[j]); });
  });
}

/* What the person has open and where they are reading in `body`, taken before `content` is drawn anew; the function it returns puts
   it back afterwards: the `details` that were open, then the scroll, kept on the card that was at the top. `live` is false where
   nothing can be measured yet (a dialog still closed): only the scroll top is kept then. Called with `false`, it puts back the
   `details` only. */
function keepReadingPlace(body, content, live = true) {
  const opened = new Map();
  if (live) content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => opened.set(`${c.dataset.cdCard}#${i}`, d.open)));
  const scroll = body.scrollTop;
  const top = body.getBoundingClientRect().top;
  const first = live ? [...content.querySelectorAll('[data-cd-card]')].find(c => c.getBoundingClientRect().bottom > top + 1) : null;
  const anchor = first && { key: first.dataset.cdCard, offset: first.getBoundingClientRect().top - top };
  return (place = true) => {
    content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => {
      const was = opened.get(`${c.dataset.cdCard}#${i}`);
      if (was !== undefined) d.open = was;
    }));
    if (!place) return;
    body.scrollTop = scroll;
    const same = anchor && [...content.querySelectorAll('[data-cd-card]')].find(c => c.dataset.cdCard === anchor.key);
    if (same) body.scrollTop += same.getBoundingClientRect().top - body.getBoundingClientRect().top - anchor.offset;
  };
}

function openPrDialog(btn) {
  const dlg = cardDialogEl();
  const rec = prSelected();
  if (!dlg || dlg.open || !rec) return;
  Object.assign(cardDialog, {
    mode: 'review', subject: selectedTaskId, pane: nav.pane, section: null, gate: null,
    opener: btn, sig: '', held: false, quiet: false, gone: false, answering: false, gateGone: false, pressing: false,
    target: null, targetAt: 0, pinned: null, dock: null, dockSig: '', groups: [], gateIds: {}, active: null,
    answered: new Map(), answeredHere: new Set(), noMore: false, token: cardDialog.token + 1,
  });
  prDialogDrawn.order = '';
  cardDialogShell('review', rec.title, prSubText(rec));
  prDialogSync(true, true);
  dlg.showModal();
  dlg.querySelector('.card-dialog-body').scrollTop = 0;
  dlg.querySelector('.card-dialog-body').focus({ preventScroll: true });
  cardDialogSpy();
}

/* `cardDialogSync` for a PR: the content is drawn from the records, the read-through and the drafts. `force` redraws although a
   box of the column has focus; `initial` is the first fill, before the dialog is open. */
function prDialogSync(force = false, initial = false) {
  const dlg = cardDialogEl();
  if (!dlg.open && !initial) return;
  const id = isPrRef(cardDialog.subject) ? prIdOfRef(cardDialog.subject) : null;
  const rec = id ? othersById(id) : null;
  // Checked first, as the task's is: what was shown stays, read-only in the sense that nothing here can reach the PR any more.
  if (!rec && others.doc) {
    cardDialog.gone = true;
    return cardDialogStrip(PR_DIALOG_GONE);
  }
  if (selectedTaskId !== cardDialog.subject || tp('task-panel').hidden) {
    cardDialog.quiet = true;
    if (dlg.open) dlg.close();
    return;
  }
  // The records have not come yet: what they lack is not known to be missing.
  if (!rec) return;
  cardDialog.gone = false;
  const title = dlg.querySelector('#card-dialog-title');
  if (title.textContent !== rec.title) title.textContent = rec.title;

  const records = othersRecords();
  const hubView = othersHubView(others.doc?.hub, records);
  const nowSecs = othersNowEst();
  const read = prReadOf(rec);
  const asking = others.asking.has(id);
  const hook = PR_HOOKS.diff;
  const groups = prDialogGroups(rec, read, records, hubView, nowSecs, {
    diff: !!hook, asking, readError: !!prReadEntry(id)?.error,
    diffFileHtml: hook ? path => hook.dialogFileHtml(rec, read, path) : null,
    diffPaths: hook && read ? hook.dialogPaths(rec, read) : null,
    diffHeadHtml: hook && read ? hook.dialogHeadHtml(rec, read) : null,
    diffLabel: hook ? path => hook.dialogLabel(rec, path) : null,
  });
  const dockHtml = prHasRead(rec, read) ? prDraftsBodyHtml(rec, read, 'dock')
    : othersOutOfReview(rec) && OTHERS_LIVE.includes(rec?.state) ? `<div class="tp-muted">${esc(PR_DIALOG_NO_SPLIT)}</div>`
    : `<div class="tp-muted">${esc(PR_DIALOG_NO_READ)}</div>${prAiButtonHtml(othersAiView(rec, records, nowSecs), hubView, asking)}`;
  prDialogPaint(groups, dockHtml, { force, initial, loading: read === undefined });
}

/* Draw `groups` into the dialog's content and `dockHtml` into its column (hidden when empty), against what was drawn last: a redraw is
   marked with what changed since, the files of the diff are drawn again, and what the person has open, where they are reading and the
   box they are typing in survive it. `force` redraws although a box of the column has focus; `initial` is the first fill, before the
   dialog is open; `loading` that the read-through is still on its way (what it brings in is the first fill of its cards, not a change).
   Shared by the review dialog and a task's own AI 下読み (task-read-view.js). */
function prDialogPaint(groups, dockHtml, { force = false, initial = false, loading = false } = {}) {
  const dlg = cardDialogEl();
  const hook = PR_HOOKS.diff;
  const html = cardDialogTaskHtml(groups);
  const col = cardDialogDockEl();
  const sig = `${html.index}\u0000${html.content}`;
  cardDialog.groups = groups;
  const contentSame = sig === cardDialog.sig && !force;
  const dockSame = dockHtml === cardDialog.dockSig && !force;
  // The column is where things are typed: it is drawn again when the box is left, never under the caret.
  const typing = !force && !prView.force && dlg.open && prBoxFocused(col);
  cardDialog.held = typing && !dockSame;
  if (contentSame && (dockSame || typing)) return;

  // Against what was last drawn, so a redraw is marked with what changed since. The first fill is not a change.
  let changes = null;
  if (!contentSame) {
    cardDialog.sig = sig;
    if (!initial && !loading) changes = prDialogWithoutDiff(cardDialogChanged(cardDialog.cardSigs, groups), groups);
    cardDialog.cardSigs = loading ? new Map() : cardDialogCardSigs(groups);
    const keys = new Set(cardDialog.cardSigs.keys());
    cardDialog.updated = new Set([...cardDialog.updated].filter(k => keys.has(k)));
    if (changes) {
      changes.cards.forEach(k => {
        cardDialog.updated.add(k);
        // A card seen a moment ago that has changed again is not seen yet: its old timer must not take the new mark.
        clearTimeout(cardDialog.seenTimers.get(k));
        cardDialog.seenTimers.delete(k);
      });
    }
  }

  const content = cardDialogContent();
  const body = dlg.querySelector('.card-dialog-body');
  // What the person has open, and where they are reading, survive the redraw.
  const restore = keepReadingPlace(body, content, dlg.open);
  const focus = cardDialogFocusOf(dlg);
  const dockKeep = dlg.open ? prFocusKeep(col) : null;

  if (!contentSame) {
    dlg.querySelector('.card-dialog-index').innerHTML = html.index;
    prDialogPatch(content, cardDialogFill(html.content, groups, () => null), groups);
    // The files that were drawn are drawn again, here, so that the position kept below is measured on the same page.
    hook?.onDrawn?.();
  }
  if (!dockSame && !typing) {
    cardDialog.dockSig = dockHtml;
    const dockScroll = col.scrollTop;
    col.innerHTML = dockHtml;
    col.hidden = !dockHtml;
    col.scrollTop = dockScroll;
    // The box an action asked for, else the control that had the keyboard; one that is gone goes to the overall comment.
    if (!prRestoreFocus(col, dockKeep, true) && dockKeep) col.querySelector('[data-pr-overall]')?.focus({ preventScroll: true });
  }
  restore(!contentSame);
  if (!contentSame) {
    cardDialogFocusBack(dlg, focus);
    if (cardDialog.pinned) cardDialog.pinnedAt = body.scrollTop;
  }
  cardDialogMarkUpdated();
  if (dlg.open) {
    if (changes?.cards.length) cardDialogShowUpdate(changes);
    else if (!contentSame) cardDialogPruneUpdate();
    hook?.applyTarget?.();
    cardDialogSpy();
  }
}

/* The dialog was closed: what was written in the column is saved now, and the panel behind it shows it. */
function prDialogClosed() {
  if (isPrRef(cardDialog.subject)) prScheduleSave(prIdOfRef(cardDialog.subject), true);
  prRedraw();
}

// The column's buttons and boxes are the panel's (`prAct`, `prInput`); 「聞く」 closes the dialog first. A task's own AI 下読み
// (task-read-view.js) has its own click, which adds what only it has.
cardDialogEl().addEventListener('click', e => {
  if (cardDialog.mode === 'review') prAct(e, { place: 'dialog' });
  else if (cardDialog.mode === 'own-read') ownReadDialogClick(e);
});
cardDialogEl().addEventListener('input', e => {
  if (cardDialogOnPr()) prInput(e);
});
cardDialogEl().addEventListener('focusout', e => {
  // A press on one of the column's buttons that was not given focus ends in `cardDialogPressEnd`, which redraws what was held.
  if (cardDialogOnPr()) prFocusOut(e, cardDialogDockEl(), () => cardDialog.held && !cardDialog.pressing, () => cardDialogSync());
});
