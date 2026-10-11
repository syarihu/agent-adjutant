/* ── 「拡大」 for a PR waiting on your review ─────────────────────────────────────────────────────────────────
   The dialog of a whole task (card-dialog.js) with a PR in it: the index on the left, the read-through in one scroll, and the
   drafts in the column on the right, where a task's gate answers sit. Its machinery is the task dialog's: `cardDialogTaskHtml`
   draws the index and the content from groups, `cardDialogChanged` marks what changed while it is open. The groups are
   pr-read.js's (`prDialogGroups`); this file draws them, keeps the column, and follows the panel. Loaded after pr-panel.js. */

const PR_DIALOG_GONE = 'パネルからこの PR がなくなりました（最後に表示した内容です）';
const PR_DIALOG_NO_READ = 'AI 下読みすると下書きの候補が入ります。';

/* A control of the column, found again after the column is drawn anew: by the first `data-pr-*` attribute it has. */
function prFocusSel(el) {
  const key = Object.keys(el?.dataset || {}).find(k => k.startsWith('pr'));
  if (!key) return null;
  const attr = key.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`);
  return `[data-${attr}="${CSS.escape(el.dataset[key])}"]`;
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
  const groups = prDialogGroups(rec, read, records, hubView, nowSecs, {
    diff: !!PR_HOOKS.diff, asking,
    diffFileHtml: PR_HOOKS.diff ? path => PR_HOOKS.diff.dialogFileHtml(rec, read, path) : null,
  });
  const html = cardDialogTaskHtml(groups);
  const col = cardDialogDockEl();
  const dockHtml = prHasRead(rec, read) ? prDraftsBodyHtml(rec, read, 'dock')
    : `<div class="tp-muted">${esc(PR_DIALOG_NO_READ)}</div>${prAiButtonHtml(othersAiView(rec, records, nowSecs), hubView, asking)}`;
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
    // A read-through still on its way: what it brings in is the first fill of its cards, not a change of them.
    const loading = read === undefined;
    if (!initial && !loading) changes = cardDialogChanged(cardDialog.cardSigs, groups);
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
  const opened = new Map();
  if (dlg.open) content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => opened.set(`${c.dataset.cdCard}#${i}`, d.open)));
  const scroll = body.scrollTop;
  const top = body.getBoundingClientRect().top;
  const first = dlg.open ? [...content.querySelectorAll('[data-cd-card]')].find(c => c.getBoundingClientRect().bottom > top + 1) : null;
  const anchor = first && { key: first.dataset.cdCard, offset: first.getBoundingClientRect().top - top };
  const focus = cardDialogFocusOf(dlg);
  const dockKeep = dlg.open ? prFocusKeep(col) : null;

  if (!contentSame) {
    dlg.querySelector('.card-dialog-index').innerHTML = html.index;
    content.replaceChildren(cardDialogFill(html.content, groups, () => null));
  }
  if (!dockSame && !typing) {
    cardDialog.dockSig = dockHtml;
    const dockScroll = col.scrollTop;
    col.innerHTML = dockHtml;
    col.hidden = false;
    col.scrollTop = dockScroll;
    // The box an action asked for, else the control that had the keyboard; one that is gone goes to the overall comment.
    if (!prRestoreFocus(col, dockKeep, true) && dockKeep) col.querySelector('[data-pr-overall]')?.focus({ preventScroll: true });
  }
  content.querySelectorAll('[data-cd-card]').forEach(c => c.querySelectorAll('details').forEach((d, i) => {
    const was = opened.get(`${c.dataset.cdCard}#${i}`);
    if (was !== undefined) d.open = was;
  }));
  if (!contentSame) {
    body.scrollTop = scroll;
    const same = anchor && cardDialogCardEl(anchor.key);
    if (same) body.scrollTop += same.getBoundingClientRect().top - body.getBoundingClientRect().top - anchor.offset;
    cardDialogFocusBack(dlg, focus);
    if (cardDialog.pinned) cardDialog.pinnedAt = body.scrollTop;
  }
  cardDialogMarkUpdated();
  if (dlg.open) {
    if (changes?.cards.length) cardDialogShowUpdate(changes);
    else if (!contentSame) cardDialogPruneUpdate();
    cardDialogSpy();
  }
}

/* The dialog was closed: what was written in the column is saved now, and the panel behind it shows it. */
function prDialogClosed() {
  if (isPrRef(cardDialog.subject)) prScheduleSave(prIdOfRef(cardDialog.subject), true);
  prRedraw();
}

// The column's buttons and boxes are the panel's (`prAct`, `prInput`); 「聞く」 closes the dialog first.
cardDialogEl().addEventListener('click', e => {
  if (cardDialog.mode === 'review') prAct(e, { inDialog: true });
});
cardDialogEl().addEventListener('input', e => {
  if (cardDialog.mode === 'review') prInput(e);
});
cardDialogEl().addEventListener('focusout', e => {
  // A press on one of the column's buttons that was not given focus ends in `cardDialogPressEnd`, which redraws what was held.
  if (cardDialog.mode === 'review') prFocusOut(e, cardDialogDockEl(), () => cardDialog.held && !cardDialog.pressing, () => cardDialogSync());
});
