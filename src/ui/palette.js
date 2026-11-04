/* The session search (Cmd/Ctrl+K): a palette over the sessions the list shows, that opens the one chosen. The first half
   is pure (what can be found, and what a query matches); the second half draws the dialog and takes the keys. */

/* Whether this is macOS: the shortcut's modifier is Cmd there (Ctrl+K is kill-to-end-of-line in a text field and in the terminal), Ctrl elsewhere. */
const IS_MAC = /Mac|iPhone|iPad/.test(navigator.platform || '');

/* Whether a keydown is the palette's shortcut: Cmd+K on macOS, Ctrl+K elsewhere, the other modifier not held, no Alt or Shift, no IME conversion open. One rule for the page and for the session terminal. */
function isPaletteShortcut(e, mac = IS_MAC) {
  if (e.type && e.type !== 'keydown') return false;
  if (e.isComposing || e.keyCode === 229) return false;
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  return !!mod && !e.altKey && !e.shiftKey && String(e.key ?? '').toLowerCase() === 'k';
}

/* The most rows drawn at once: a query narrows the rest. */
const PALETTE_SHOWN = 100;

/* The last number in a string ('owner/repo#12', '.../issues/12') as the issue's number, or ''. */
const paletteNumber = v => /(\d+)\s*$/.exec(String(v ?? ''))?.[1] || '';

/* What a query is looked for in: every word that names the session, lower-cased and joined. */
function paletteHaystack(parts) {
  return parts.filter(x => x != null && x !== '').map(x => String(x).toLowerCase()).join('\n');
}

/* One candidate. `row` is what the list opens (resident server), `sid` the session's ref (a board served alone).
   `parent` is { key, number, title } of the parent issue the session works under, if any; `hub` the hub that runs it. */
function paletteEntry({ key, board = null, ref = null, sid = null, row = null, repoIdx = 0, s, st, data, task = null, nwo = '', parent = null, hub = null }) {
  const label = sessionLabel(s, false, data);
  const title = task?.title || label.text;
  const num = paletteNumber(task?.issue) || paletteNumber(task?.issueUrl);
  const worktree = baseName(s.worktree);
  const sub = [label.tag || (task?.id && task.id !== title ? task.id : ''), s.kind !== 'hub' ? s.branch : '', nwo].filter(Boolean);
  return {
    key, board, ref, sid, row, repoIdx, title, sub: sub.join(' · '), st, isHub: s.kind === 'hub',
    hay: paletteHaystack([
      title, label.text, label.tag, task?.id, task?.issue, task?.issueUrl, num, num && `#${num}`,
      s.branch, nwo, parent?.key, parent?.number && `#${parent.number}`, parent?.title,
      hub?.key, hub?.title, worktree, task?.worktreeName,
    ]),
  };
}

/* The order of the candidates. Sessions that wait on the person (the pair my-work.js's WORKS_ON_PERSON names) come first, 確認待ち before
   入力待ち (STATE_ORDER), each in the order the work document gives them. The rest follow, repository by repository in the document's order and
   within one by `workRowOrder` (the task's id), which is the order 「いまの仕事」 sorts a group with; its grouping by parent is not copied.
   On a board served alone (no `row`) the rest keep the order of the sessions. The sort is stable, so ties are never reshuffled. */
const paletteRank = e => e.st === 'waiting' || e.st === 'permission' ? STATE_ORDER[e.st] : 2;
function paletteCompare(a, b) {
  const d = paletteRank(a) - paletteRank(b);
  if (d || paletteRank(a) < 2 || !a.row || !b.row) return d;
  return a.repoIdx - b.repoIdx || workRowOrder(a.row, b.row);
}
const paletteOrder = entries => [...entries].sort(paletteCompare);

/* The repositories of the selected organisation (or in none the page lists) that the work document could not read, by name. Their rows are
   empty, so without this an unreadable repository looks like one with no sessions. Only the resident server has a work document. */
function paletteUnreadRepos(doc, scope) {
  return (doc?.repos || []).filter(r => r.error && orgInScope(scope, r.nwo)).map(r => r.nwo);
}

/* The words under the results: what the query found (or why nothing is shown), and, apart from it, that the work document could not be
   refreshed and the list is the last one read. `notice` (loading, or failed with nothing read) stands alone. */
function paletteStatusText({ notice, total, found, drawn, stale, unread = [] }) {
  if (notice) return notice;
  const rest = found - drawn;
  const result = !total ? 'セッションがありません' : !found ? '該当するセッションがありません' : rest > 0 ? `ほか ${rest} 件があります。言葉を足して絞り込んでください` : '';
  // A repository that could not be read has no rows: that is not "no sessions" in it.
  const names = unread.length > 3 ? `${unread.slice(0, 3).join(', ')} ほか ${unread.length - 3} 件` : unread.join(', ');
  return [result, stale ? '最新の状態を読めませんでした（前回の一覧です）' : '', unread.length ? `読めなかったリポジトリがあります: ${names}` : ''].filter(Boolean).join(' ');
}

/* The key to highlight: `active` while it is one of the rows drawn, else the first drawn row (never one that is listed but not drawn). */
function paletteActiveKey(drawn, active) {
  return drawn.some(e => e.key === active) ? active : drawn[0]?.key ?? null;
}

/* The candidates, in paletteOrder: the sessions the list shows. On the resident server that is the work document's rows of the organisation selected
   (a failed or missing document has none: the caller says so), on a board served alone the sessions the list would hold.
   `src` is { multi, doc, scope, state }. A key (board and ref, never a session id: two boards can share one) is listed once. */
function paletteEntries({ multi, doc, scope, state: st0 }) {
  const out = [];
  const seen = new Set();
  const add = e => { if (!seen.has(e.key)) { seen.add(e.key); out.push(e); } };
  if (multi) {
    for (const r of workRows(doc)) {
      if (!orgInScope(scope, r.nwo)) continue;
      const hub = r.hubSlug && (r.repo.hubs || []).find(h => h.slug === r.hubSlug && h.parent) || null;
      const pkey = r.task?.parent || hub?.key || '';
      const parent = pkey ? (r.repo.parents || []).find(p => p.key === pkey) || { key: pkey } : null;
      add(paletteEntry({ key: r.key, board: r.board, ref: r.ref, row: r, repoIdx: (doc.repos || []).indexOf(r.repo), s: r.s, st: r.st, data: r.data, task: r.task, nwo: r.nwo, parent, hub }));
    }
    return paletteOrder(out);
  }
  for (const s of st0.sessions || []) {
    const st = sessionState(s, st0);
    if (!WORK_LISTED.includes(st)) continue;
    const task = s.task ? (st0.tasks || []).find(t => t.id === s.task) || null : null;
    const pkey = task?.parent || '';
    const parent = pkey ? (st0.parents || []).find(p => p.key === pkey) || { key: pkey } : null;
    const ref = sessionRef(s);
    add(paletteEntry({ key: `self/${ref}`, sid: ref, s, st, data: st0, task, nwo: st0.repo || '', parent }));
  }
  return paletteOrder(out);
}

/* The entries whose haystack holds every word of `query` (any case, any order). An empty query keeps them all; the order is the candidates' (paletteOrder). */
function paletteFilter(entries, query) {
  const words = String(query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
  return words.length ? entries.filter(e => words.every(w => e.hay.includes(w))) : entries;
}

/* ── drawing and keys ── */

const pal = { entries: [], shown: [], active: null, prev: null, sig: '', timer: null };
const palEl = id => document.getElementById(id);

const paletteSource = () => ({ multi: multiBoard, doc: work.doc, scope: orgScope(), state });

/* What is said instead of the list, when it is not an answer: the work document not read yet or failed (a failed read is not "no results"). */
function paletteNotice() {
  if (!multiBoard) return '';
  if (!work.doc) return work.error ? `読み込めませんでした: ${work.error}` : '読み込み中…';
  return '';
}

function paletteRowHtml(e, i) {
  const [glyph, glyphLabel] = WORK_GLYPH[e.st] || WORK_GLYPH.idle;
  const word = ROW_LABEL[e.st] || '';
  return `<li role="option" id="pal-opt-${i}" class="pal-opt" data-key="${esc(e.key)}" aria-selected="${e.key === pal.active}">`
    + `<span class="material-symbols-outlined pal-glyph ${esc(e.st)}" role="img" aria-label="${esc(glyphLabel)}" title="${esc(glyphLabel)}">${glyph}</span>`
    + `<span class="pal-main"><span class="pal-title">${esc(e.title || e.sub || e.key)}</span>${e.sub ? `<span class="pal-sub">${esc(e.sub)}</span>` : ''}</span>`
    + `<span class="pal-state">${esc(word)}</span></li>`;
}

function setPaletteActive(key, scroll = true) {
  pal.active = key;
  const input = palEl('palette-input');
  let activeId = '';
  palEl('palette-list').querySelectorAll('[role="option"]').forEach(el => {
    const on = el.dataset.key === key;
    el.setAttribute('aria-selected', String(on));
    if (on) { activeId = el.id; if (scroll) el.scrollIntoView({ block: 'nearest' }); }
  });
  if (activeId) input.setAttribute('aria-activedescendant', activeId); else input.removeAttribute('aria-activedescendant');
}

/* Draw the results of the query as it stands. The input is never touched, so what is typed stays; the highlight stays on its key while that is listed. */
function renderPalette() {
  const list = palEl('palette-list'), status = palEl('palette-status');
  pal.shown = paletteFilter(pal.entries, palEl('palette-input').value);
  const drawn = pal.shown.slice(0, PALETTE_SHOWN);
  pal.active = paletteActiveKey(drawn, pal.active);
  list.innerHTML = drawn.map(paletteRowHtml).join('');
  status.textContent = paletteStatusText({ notice: paletteNotice(), total: pal.entries.length, found: pal.shown.length, drawn: drawn.length, stale: multiBoard && !!work.error, unread: multiBoard ? paletteUnreadRepos(work.doc, orgScope()) : [] });
  setPaletteActive(pal.active, false);
  const el = list.querySelector('[aria-selected="true"]');
  if (el) el.scrollIntoView({ block: 'nearest' });
}

/* A poll may have changed what can be found: the candidates are built again and the results redrawn only if they differ. */
function syncPalette() {
  if (!palEl('palette').open) return;
  pal.entries = paletteEntries(paletteSource());
  const sig = JSON.stringify([pal.entries.map(e => [e.key, e.title, e.sub, e.st, e.hay]), paletteNotice(), multiBoard && work.error, multiBoard && paletteUnreadRepos(work.doc, orgScope())]);
  if (sig === pal.sig) return;
  pal.sig = sig;
  renderPalette();
}

function openPalette() {
  const dlg = palEl('palette');
  if (dlg.open) { palEl('palette-input').focus(); palEl('palette-input').select(); return; }
  pal.prev = document.activeElement;
  palEl('palette-input').value = '';
  pal.active = null;
  pal.entries = paletteEntries(paletteSource());
  pal.sig = '';
  dlg.showModal();
  syncPalette();
  palEl('palette-input').focus();
  pal.timer = setInterval(syncPalette, 1000);
}

/* Open the palette for the shortcut `e` unless another dialog is open (then the key is left alone). Whether `e` is the shortcut is isPaletteShortcut's to say. */
function openPaletteFromKey(e) {
  if (document.querySelector('dialog[open]:not(#palette)')) return false;
  e.preventDefault();
  openPalette();
  return true;
}

function closePalette() {
  const dlg = palEl('palette');
  if (dlg.open) dlg.close();
}

/* Open the session as a click on its row of the list does: selected in the work view on the resident server (the repository
   filter is dropped when it would hide the row), the task panel over the board on a board served alone. A session that is not
   running is only selected: its panel offers 再開, nothing is resumed or opened from here. */
function choosePaletteEntry(key) {
  const e = pal.entries.find(x => x.key === key);
  if (!e) return;
  pal.prev = null;
  closePalette();
  if (e.row) {
    const choice = workRepoChoice();
    if (choice && e.row.nwo.toLowerCase() !== choice) { prefs.workRepo = ''; savePrefs(); renderWorkBadge(); }
    selectWorkRow(work.rowsByKey.get(e.key) || e.row);
  } else openSessionRef(e.sid);
}

function movePaletteActive(step) {
  const keys = pal.shown.slice(0, PALETTE_SHOWN).map(e => e.key);
  if (!keys.length) return;
  const i = keys.indexOf(pal.active);
  setPaletteActive(keys[i < 0 ? (step > 0 ? 0 : keys.length - 1) : (i + step + keys.length) % keys.length]);
}

palEl('palette-input').addEventListener('input', renderPalette);
palEl('palette-input').addEventListener('keydown', e => {
  // Enter that confirms an IME conversion must not choose.
  if (e.isComposing || e.keyCode === 229) return;
  if (e.key === 'ArrowDown') { e.preventDefault(); movePaletteActive(1); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); movePaletteActive(-1); }
  else if (e.key === 'Enter') { e.preventDefault(); if (pal.active) choosePaletteEntry(pal.active); }
});
palEl('palette-list').addEventListener('click', e => {
  const el = e.target.closest('[role="option"]');
  if (el) choosePaletteEntry(el.dataset.key);
});
palEl('palette-list').addEventListener('mousemove', e => {
  const el = e.target.closest('[role="option"]');
  if (el && el.dataset.key !== pal.active) setPaletteActive(el.dataset.key, false);
});
// A click on the backdrop is a click on the dialog itself.
palEl('palette').addEventListener('click', e => { if (e.target === e.currentTarget) closePalette(); });
palEl('palette').addEventListener('close', () => {
  clearInterval(pal.timer);
  pal.timer = null;
  const prev = pal.prev;
  pal.prev = null;
  if (prev?.isConnected && typeof prev.focus === 'function') prev.focus();
});
