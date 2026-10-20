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

/* One press: the clean-up that refuses to lose anything, on the board the row belongs to (`facts`: judged on the work document's facts, see below). If work turned up since the read, the row
   says what and the dialog opens on the way past it; if it went through, the row is gone from the list. */
async function workCleanUp(r, { facts = false } = {}) {
  const key = r && workCleanable(r) ? workCleanKey(r) : null;
  if (!key || workClean.busy.has(key)) return;
  // Never on a read that did not say the worktree is clean. The TODO's and the terminal's button (`facts`) have the document's facts
  // instead of the row's own read, and the server looks again either way.
  if (facts) {
    if (workMergedAsk(r)?.ask !== 'clean') return;
  } else if (!workCleanFacts(r.s.uncommitted, workClean.git.get(key)?.git).ok || workClean.git.get(key)?.blocked) return;
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

/* ── The board asks about a merged session by itself ──────────────────────────────────────────────────────────────────────────
   The row's own line above reads git per row. The TODO and the line over the terminal have only the work document, which carries the
   same facts for every session (`uncommitted`, `unpushed`, and `cleanupKeptAt` once 「残す」 was pressed), so the question is built from
   those: a merged worker session with nothing to lose is asked about (「片付ける」 / 「残す」), one with something to lose gets a warning
   line instead and nothing to press but a look in the terminal. The key of a question is `clean:<board>/<session id>`, the board being the
   slug the server names the row by (`entry.board`, the row's `hubSlug`), so two boards' sessions never share one. */

/* What the page did about the questions, by that key: the ones with a request out, and the ones 「残す」 was pressed on, hidden until
   the work document says it too (`workAskPrune`). */
const workAsk = { busy: new Set(), kept: new Set() };
const workAskKey = (board, sessionId) => `clean:${board}/${sessionId}`;

/* What a merged session gets from the document alone, as { ask, files, unpushed, number, title }, or null: `ask` is 'clean' (nothing to
   lose) or 'dirty' (work would be lost). `r` is a row (`{ s, task }`) or an entry (`{ session, task }`). A session that is not a worker's with a
   worktree, whose PR is not merged, or whose facts are not known is null: a read that failed or has not come is never "0". */
function workMergedAsk(r) {
  const s = r?.s ?? r?.session;
  const task = r?.task || null;
  if (!s || r.isHub || s.kind !== 'worker' || !s.id || !s.worktree || !workMerged({ task, s })) return null;
  const u = s.uncommitted;
  if (s.uncommittedError || !u || !Number.isFinite(u.files) || !Number.isFinite(u.untracked) || !Number.isFinite(s.unpushed)) return null;
  const files = u.files + u.untracked;
  const number = task ? prRefNumber(task.pr) : s.branchPr?.number;
  return { ask: files === 0 && s.unpushed === 0 ? 'clean' : 'dirty', files, unpushed: s.unpushed, number: number ? String(number) : '',
    title: task?.title || s.taskTitle || s.title || baseName(s.worktree) || s.id };
}

/* Whether a gate waits for the entry's session: the entry holds one (a task's, or the session's own `waiting`, which a gate answered
   on this page no longer counts). Then only the gate is asked about. */
const workEntryGated = (e, isDone = () => false) => (e.gates || []).length > 0 || (!!e.session?.waiting && !isDone(`${e.session.waiting.slug}/${e.session.waiting.id}`));

/* What the board asks about the entry's session, or null: `workMergedAsk` for one no gate waits for. 「残す」 (the record's
   `cleanupKeptAt`, or the key in `kept` until the document says it) hides the 'clean' question; the 'dirty' line asks nothing, so it stays. */
function workMergedAskOf(e, { isDone = () => false, kept = new Set() } = {}) {
  const a = e?.session ? workMergedAsk(e) : null;
  if (!a || workEntryGated(e, isDone)) return null;
  if (a.ask === 'clean' && (e.session.cleanupKeptAt != null || kept.has(workAskKey(e.board, e.session.id)))) return null;
  return a;
}

/* Drop from `kept` the sessions the document now carries 「残す」 for, and those it no longer lists. */
function workAskPrune(kept, doc) {
  // A document not read yet says nothing about a session.
  if (!kept.size || !doc) return;
  const held = new Map();
  for (const repo of doc.repos || []) for (const r of repo.rows || []) held.set(workAskKey(r.board, r.session.id), r.session);
  for (const key of [...kept]) if (!held.has(key) || held.get(key).cleanupKeptAt != null) kept.delete(key);
}

/* The question, in words. In the TODO it names the session; over its terminal, which names it already, it does not. */
const workMergedAskText = (a, titled = false) => `${titled ? `${a.title}: ` : ''}${a.number ? `PR #${a.number} ` : ''}マージ済み。片付けてセッションを閉じる？`;

/* The one-line question as the quick row's shape: icon, text, the two chips, the buttons. The 'dirty' line has a warning and a button
   that looks in the terminal, and none that removes anything. `text` is the question when it names the session. `busy` disables the buttons while a request of theirs is out. */
function workMergedAskHtml(a, { board, sid, text = workMergedAskText(a), busy = false } = {}) {
  const at = `data-wk-ask-board="${esc(board)}" data-wk-ask-sid="${esc(sid)}"`;
  const button = (act, cls, label) => `<button type="button" class="${cls} qc-quick-act" ${at} data-wk-ask-act="${act}"${busy ? ' disabled' : ''}>${label}</button>`;
  if (a.ask === 'dirty') {
    return `<div class="qc-quick wk-merged-ask bad">${qcIcon('warning')}<b class="qc-quick-text">${esc(`マージ済みですが、消えると困る変更があります（未 push ${a.unpushed} · 未コミット ${a.files}）`)}</b>`
      + `<span class="qc-spacer"></span>${button('check', 'btn-m3-text', 'ターミナルで確かめる')}</div>`;
  }
  return `<div class="qc-quick wk-merged-ask">${qcIcon('cleaning_services')}<b class="qc-quick-text">${esc(text)}</b>`
    + '<div class="qc-chips"><span class="qc-chip ok">未コミット 0</span><span class="qc-chip ok">未 push 0</span></div>'
    + `<span class="qc-spacer"></span><span class="qc-quick-btns">${button('cleanup', 'btn-m3-primary', '片付ける')}${button('keep', 'btn-m3-tonal', '残す')}</span></div>`;
}

/* 「残す」: hide the question on the server (the worker's record), and here until the document says it. A failure is said and leaves the
   question where it was. */
async function workKeep(r) {
  const key = workAskKey(r.hubSlug, r.s.id);
  const line = `残す (${baseName(r.s.worktree) || r.s.id})`;
  try {
    await boardApi(`/b/${r.board}`, `/api/sessions/${enc(r.s.id)}/keep`, { method: 'POST', body: '{}' });
    workAsk.kept.add(key);
    note(line, false, '片付けずに残します');
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

/* A press on a question's button (the TODO's or the terminal's): the row is found among the ones the page builds from the document, by
   the board and the session id the button carries. A second press while one is out does nothing. 「片付ける」 is the row's clean-up
   run on the document's facts; 「ターミナルで確かめる」 only looks, and is the caller's. */
async function workAskPress(act, board, sid) {
  const key = workAskKey(board, sid);
  if (workAsk.busy.has(key)) return;
  const r = workRows().find(x => x.hubSlug === board && x.s.id === sid);
  if (!r) return note(act === 'keep' ? '残す' : '片付ける', true, 'このセッションは、もう一覧にありません');
  workAsk.busy.add(key);
  try {
    if (act === 'cleanup') await workCleanUp(r, { facts: true });
    else if (act === 'keep') await workKeep(r);
  } finally {
    workAsk.busy.delete(key);
  }
}
