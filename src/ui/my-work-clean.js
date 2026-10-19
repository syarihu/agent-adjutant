/* ── 「片付ける」 on a row whose PR is merged ────────────────────────────────────────────────────────
   The row shows, on a line of its own, what removing its worktree would lose, and the button when nothing would be. What it knows
   comes from the session's own `/git` read, taken when the row is drawn and kept for a minute; the server checks again before it
   removes anything, and the dialog of the board's sessions is where a clean-up that would lose work is forced. */

/* What was read per merged row, by `<board>/<session id>`: { at, git, error, reading, blocked, failed }. */
const workClean = { git: new Map(), busy: new Set() };
const WORK_CLEAN_TTL = 60000;
const workCleanKey = r => `${r.board}/${r.s.id}`;

/* What there is to lose, as { files, unpushed, ok }: the changed and untracked files and the commits no remote has. Only a git reply
   that says both is `ok` with nothing in either; the row's own counts stand in for the files while the reply is missing, and never
   make it `ok`. */
function workCleanFacts(uncommitted, git) {
  const count = u => u ? (Number(u.files) || 0) + (Number(u.untracked) || 0) : null;
  const read = git?.uncommitted ? count(git.uncommitted) : null;
  const up = git?.unpushed?.count;
  const unpushed = Number.isFinite(up) ? up : null;
  return { files: read ?? count(uncommitted), unpushed, ok: read === 0 && unpushed === 0 };
}

/* The line under a merged row, and its button. `entry` is what `workClean.git` holds for the row (none: not read yet). The button is
   only for a read that found nothing to lose, and it is the row button's sibling, never inside it. */
function workCleanLineHtml(r, entry, busy = false) {
  // A row that cannot be cleaned up says so in the error colour, with a warning icon.
  const line = (text, button = '', bad = false) => `<span class="wk-clean${bad ? ' bad' : ''}"><span class="wk-clean-line">${bad ? '<span class="material-symbols-outlined" aria-hidden="true">warning</span>' : ''}${text}</span>${button}</span>`;
  if (!entry || (entry.reading && !entry.git && !entry.blocked)) return line('確認しています…');
  if (entry.failed) return line(esc(`片付けられませんでした: ${entry.failed}`), '', true);
  if (entry.error) return line('git の状態を確認できません', '', true);
  const f = workCleanFacts(r.s?.uncommitted, entry.git);
  const counts = `未コミット ${f.files ?? '?'} · 未 push ${f.unpushed ?? '?'}`;
  if (entry.blocked && !entry.git) return line(esc(`片付けられません: ${(entry.reasons || []).map(x => x.detail).join('、')}`), '', true);
  if (entry.blocked || !f.ok) return line(`${esc(counts)} — 片付けられません`, '', true);
  const name = baseName(r.s?.worktree) || r.s?.id || '';
  const label = `片付ける（${name}）`;
  return line(esc(counts), `<button type="button" class="wk-clean-btn" data-wk-clean="${esc(r.key)}"${busy ? ' disabled' : ''} aria-label="${esc(label)}" title="${esc('worktree とブランチを消します')}"><span class="material-symbols-outlined" aria-hidden="true">cleaning_services</span>${busy ? '片付けています…' : '片付ける'}</button>`);
}

/* The row's own line, from what is held. */
const workCleanRowHtml = r => workCleanLineHtml(r, workClean.git.get(workCleanKey(r)), workClean.busy.has(workCleanKey(r)));

/* Whether a row gets the line: a merged one that is a worker's session with a worktree. */
const workCleanable = r => !r.isHub && !r.turn && workMerged(r) && r.s?.kind === 'worker' && !!r.s.id && !!r.s.worktree;

/* Read the row's git state unless it was read within the minute or a read is out; the list is drawn again with the answer. */
function workCleanWant(r) {
  const key = workCleanKey(r);
  const held = workClean.git.get(key);
  if (held?.reading || (held && Date.now() - held.at < WORK_CLEAN_TTL)) return;
  workClean.git.set(key, { ...held, at: held?.at ?? 0, reading: true, error: null, failed: null, blocked: false });
  const done = entry => {
    workClean.git.set(key, { at: Date.now(), reading: false, ...entry });
    if (view === 'work') renderWorkList();
  };
  boardApi(`/b/${r.board}`, `/api/sessions/${enc(r.s.id)}/git`).then(
    git => done({ git, error: null }),
    e => done({ git: null, error: e.message || String(e) }));
}

/* A press on a row's button: the row is found by the key the button carries, in the rows as last drawn. */
function workCleanClicked(button, rowsByKey) {
  const r = rowsByKey.get(button.dataset.wkClean);
  return r && !button.disabled && workCleanUp(r);
}

/* One press: the clean-up that refuses to lose anything, on the board the row belongs to. If work turned up since the read, the row
   says what and the dialog opens on the way past it; if it went through, the row is gone from the list. */
async function workCleanUp(r) {
  const key = r && workCleanable(r) ? workCleanKey(r) : null;
  if (!key || workClean.busy.has(key)) return;
  // Never on a read that did not say the worktree is clean.
  if (!workCleanFacts(r.s.uncommitted, workClean.git.get(key)?.git).ok || workClean.git.get(key)?.blocked) return;
  const base = `/b/${r.board}`;
  const line = `worktree を片付ける (${baseName(r.s.worktree) || r.s.id})`;
  workClean.busy.add(key);
  renderWorkList();
  try {
    const data = await boardApi(base, `/api/sessions/${enc(r.s.id)}/cleanup`, { method: 'POST', body: '{}' });
    if (data.removed) {
      const { text, bad } = cleanupSummary(data);
      note(line, bad, text);
      workClean.git.delete(key);
      if (workIsSelected(r)) go({ board: 'all', view: 'work', task: null, pane: 'detail' });
      refreshWork(true);
    } else {
      workClean.git.set(key, { at: Date.now(), git: data.git || null, reasons: data.reasons || [], blocked: true });
      note(line, false, '失われる作業があるため削除しませんでした');
      openCleanupDialog(r.s, base, { blocked: true });
    }
  } catch (e) {
    note(`${line} → ${e.message}`, true);
    workClean.git.set(key, { at: Date.now(), failed: e.message });
  } finally {
    workClean.busy.delete(key);
    if (view === 'work') renderWorkList();
  }
}
