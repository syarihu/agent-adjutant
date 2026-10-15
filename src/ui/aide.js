/* ── 副官: the middle of 「いまの仕事」 when nothing is selected ─────────────────────────────────────
   The aide's terminal on the left and the display area on the right (the pure half is aide-display.js), the pinned 「副官」 row of
   the list, the bar and the count that bring the person back from a session it sent them to, and the buttons that ask it
   something. The address is `/?view=work` with no task: no new parameter, so Back from a session lands here.

   Showing the pane never starts the aide: Claude Code asks once whether to trust its directory, and that question needs a
   person looking. Only the 「副官」 row, 「副官を起動」 and 「副官に聞く」 call `POST /api/aide/open`. `GET /api/aide` runs a process
   check, so it is read only while the pane is on screen (core.js `refresh`). */

const aide = {
  view: null,        // GET /api/aide, as last read; kept when a read fails
  error: null,
  busy: false,
  again: false,      // a forced round asked for while one was out
  reads: 0,          // reads of /api/aide sent so far
  readOk: 0,         // which of them was the last that worked; a queued request is typed only from one sent after it
  askAfter: 0,       // how many reads had been sent when the queued request was made
  display: aideEmptyDisplay(),
  gateTitles: new Map(), // gate ref -> its title as last read, so a part of a gate that is gone still says which it was
  boardSeq: {},      // board slug -> reads of its state sent so far
  boardApplied: {},  // board slug -> the newest of them that was applied; an older answer arriving later is dropped
  opening: false,
  resetting: false,
  wantOpen: false,   // the person asked for it to be started; acted on when a read says it can be
  pendingAsk: null,  // a request typed once the aide can take it (aideFlushAsk)
  from: null,        // { board, ref, title }: the row the aide sent the person to, for the way back; page memory, not the address
  seen: null,        // the keys of the waits the pane last showed, for 「待ちが N 件増えました」; null before it was shown
  boards: {},        // board slug -> { data, at, error }: the state of each board a gate part or a wait names, tagged as mergeStates does
  local: null,       // { part, serverTop, topic }: the part this page put on top of the aide's (aideDisplayFollowed); page memory only
  prs: new Map(),    // PR id -> { rec, read, diff, resolved, settled, error, at, recKey, readKey, diffKey }: what the parts that name a PR read of it (records only)
  taskAis: new Map(), // `board/task` -> { pr, record, at, error }: a task's PR and the record of it, as `GET /api/tasks/<id>/ai` says
  taskSeq: {},       // `board/task` -> reads of its document sent so far; and the newest applied
  taskApplied: {},
  prSeq: {},         // PR id -> reads of it sent so far; and, as boardSeq is, the newest applied
  prApplied: {},
  gateDiffs: new Map(), // gate ref -> { diff, files }: a gate's diff parsed once, not at every redraw
  drafts: new Map(), // gate ref -> a comment typed in a part, kept across redraws (like cardDialogDrafts)
  held: false,       // a redraw of the display area is waiting for the comment box that has the focus to lose it
};
/* The aide's terminal: a slot like the middle's (`workTerm`), connected through the aide's own socket and not a board's. */
const aideTerm = { host: () => wk('aide-term-host'), redraw: () => renderAide(), base: () => '',
  taskId: null, sessionId: null, term: null, ended: null, endReason: null, reconnect: false,
  // The terminal takes the keyboard only when the person asked for the aide, not when a poll mounts it.
  focus: false };
/* The markup each part of the pane was last given, so a poll that changes nothing leaves the focus and an open <details> alone. */
const aideDrawn = {};
function setAidePart(name, el, html) {
  if (!el || aideDrawn[name] === html) return;
  aideDrawn[name] = html;
  el.innerHTML = html;
}

/* The display area is drawn when its markup changed apart from the times in it and the text typed into a comment box (that is
   kept in `aide.drafts`, so the markup of a box is not what makes it differ). While a comment box has the focus the redraw
   waits, and goes when the focus leaves, so that what is being typed is not thrown away. */
function setAideDisplay(el, html) {
  if (!el) return;
  const sig = html.replace(/(<textarea[^>]*>)[^<]*(<\/textarea>)/g, '$1$2');
  const typing = !!document.activeElement?.matches?.('.gate-comment') && el.contains(document.activeElement);
  // The times in it stay fresh: they are left out of the comparison only while a comment is being typed, when a redraw would
  // cost what is being typed and the minute turning over is no reason for one.
  if (typing ? timeFree(aideDrawn.display ?? '') === timeFree(sig) : aideDrawn.display === sig) {
    aide.held = false;
    return;
  }
  if (typing) {
    aide.held = true;
    return;
  }
  aide.held = false;
  aideDrawn.display = sig;
  const top = el.scrollTop;
  el.innerHTML = html;
  el.scrollTop = top;
}

/* What the display area shows: the state the aide's adjutant_show tool left, which the resident reads from its file. */
async function loadAideDisplay() {
  return boardApi('', '/api/aide/display');
}

/* ── what the work document says ── */

const aideCurrentWaits = () => (work.doc ? aideWaits(workEntries(work.doc, key => answeredGates.has(key))) : []);

const aideRowTitle = r => r.task?.title || (r.turn
  ? (r.entry.session ? sessionLabel(r.entry.session, false, r.data).text : r.entry.gateTitle || r.ref)
  : sessionLabel(r.s, false, r.data).text);

/* The rows of 新着 as the list draws them, and the first of them with what it is in. A hub is no row of it. */
function aideNews() {
  if (!work.doc) return null;
  const { rows, turns } = workListRows();
  const at = r => { const i = work.newOrder.indexOf(r.id); return i < 0 ? Infinity : i; };
  const fresh = [...rows, ...turns].filter(r => r.cls === 'new' && !r.isHub).sort((a, b) => at(a) - at(b));
  if (!fresh.length) return null;
  const r = fresh[0];
  const gate = (r.live || []).find(i => i.kind === 'gate');
  const pr = (r.live || []).find(i => i.kind === 'pr');
  const kindLabel = gate ? kindOf(gate.gate)[0] : '';
  const what = kindLabel || (pr && WORK_PR_NOW[pr.turn]) || WORK_GLYPH[r.st]?.[1] || '';
  return { count: fresh.length, first: { title: aideRowTitle(r), what, key: r.key, nwo: r.nwo, taskId: r.task?.id || null, kindLabel } };
}

/* ── what the parts show: the content of the boards ── */

/* How long what was read of a board is kept before a poll reads it again. */
const AIDE_BOARD_FRESH_MS = 10000;

const aideTagged = (list, slug) => (list || []).map(x => ({ ...x, _slug: slug, _base: `/b/${slug}` }));

/* What the display area shows now: the aide's parts with the one this page put on top, or the list of waits when there is
   nothing at all. */
const aideShown = () => aideDisplayWithWaits(aideDisplayFollowed(aide.display, aide.local), aideCurrentWaits());

/* What the parts on show need read: `boards` (a gate part's own, a task's, a session's, and every board that has a wait when the list of
   waits is on), `taskAis` (the task a gate, or a PR shown by its task, is of: `{ board, task }` each) and `prs` (PR id -> `{ read, diff }`,
   what of the PR is wanted: its record always, its read-through for a read, a diff of the PR and the annotations of a gate's 「根拠」, its
   stored diff for a read and a diff). Nothing here reaches GitHub: they are the resident's records. */
function aideWanted() {
  const shown = aideShown();
  const boards = new Set();
  const taskAis = new Map();
  const prs = new Map();
  const wantTask = (board, task) => { if (board && task) taskAis.set(`${board}/${task}`, { board, task }); };
  const wantPr = (id, what = {}) => { if (id) prs.set(id, { ...(prs.get(id) || {}), ...what }); };
  for (const p of shown.parts) {
    const ref = p.ref || {};
    if (p.kind === 'gate' && ref.board) {
      boards.add(String(ref.board));
      const g = (aide.boards[ref.board]?.data?.gates || []).find(x => x.id === ref.id);
      if (g?.task) {
        wantTask(ref.board, g.task);
        // The annotations of 「根拠」 are the read-through of the task's PR, when it has one.
        const ai = aide.taskAis.get(`${ref.board}/${g.task}`);
        if (ai?.pr && ai.record?.ai?.sha) wantPr(ai.pr, { read: true });
      }
    }
    if (['task', 'session'].includes(p.kind) && ref.board) boards.add(String(ref.board));
    if (p.kind === 'pr') {
      if (ref.pr) wantPr(ref.pr);
      else if (ref.board) { boards.add(String(ref.board)); wantTask(ref.board, ref.id); }
    }
    if (p.kind === 'read') wantPr(ref.pr, { read: true, diff: true });
    if (p.kind === 'diff') {
      if (ref.pr) wantPr(ref.pr, { read: true, diff: true });
      else if (ref.board) boards.add(String(ref.board));
    }
    if (p.kind === 'waits') for (const w of aideCurrentWaits()) if (w.board) boards.add(w.board);
  }
  // The board of a dialog the aide opened stays read as long as the dialog is open.
  if (cardDialog.scope && cardDialogEl().open) boards.add(cardDialog.scope.board);
  return { boards, taskAis, prs };
}

/* Whether the entry of a PR holds what `what` asks for: the read-through when wanted (`undefined` is never fetched), the diff when wanted. */
const aideHasWanted = (e, what) => {
  if (!e || !e.settled) return false;
  // What it holds must be of the record's current head: a record read again at another head makes an old read-through or diff stale.
  const key = e.rec ? prReadKey(e.rec) : 'no-record';
  return (!what.read || (e.read !== undefined && e.readKey === key)) && (!what.diff || !e.read || (e.diff.state !== 'loading' && e.diffKey === key));
};

/* Whether something read at `at` is recent enough not to be read again. */
const aideIsFresh = (at, force) => !force && at != null && Date.now() - at < AIDE_BOARD_FRESH_MS;

/* Read the state of each wanted board (without its sessions, as 「すべて」 does), unless it was read a moment ago and `force`
   does not say otherwise. A board that cannot be read keeps what was read of it, with the reason; one never read has only the reason.
   Then what the parts name of the records (`loadAideRecords`), which may depend on a board. */
async function loadAideBoards(force = false) {
  const at = Date.now();
  const stale = [...aideWanted().boards].filter(slug => force || !aide.boards[slug] || at - aide.boards[slug].at >= AIDE_BOARD_FRESH_MS);
  await Promise.all(stale.map(async slug => {
    const n = aide.boardSeq[slug] = (aide.boardSeq[slug] || 0) + 1;
    // A read sent before a newer one was applied (or before an answer took a gate off) must not put it back.
    const current = () => {
      if (n < (aide.boardApplied[slug] || 0)) return false;
      aide.boardApplied[slug] = n;
      return true;
    };
    try {
      const next = await boardApi(`/b/${slug}`, '/api/state?sessions=0');
      if (!current()) return;
      // The lists `mergeStates` (core.js) tags with their board, since the cards match on `_slug`.
      const tagged = Object.fromEntries(['tasks', 'gates', 'waits', 'parents', 'workers', 'hubs'].map(k => [k, aideTagged(next[k], slug)]));
      aide.boards[slug] = { data: { ...next, ...tagged }, at: Date.now(), error: null };
      renderAide();
    } catch (e) {
      if (!current()) return;
      aide.boards[slug] = { data: aide.boards[slug]?.data || null, at: Date.now(), error: e.message };
      renderAide();
    }
  }));
  await loadAideRecords(force);
  // A dialog the aide opened follows what was read.
  if (cardDialog.scope) cardDialogFollow();
}

/* The task documents first (they say which PR a task is of), then the PRs the parts and those documents name. */
async function loadAideRecords(force = false) {
  const tasks = [...aideWanted().taskAis.values()].filter(t => !aideIsFresh(aide.taskAis.get(`${t.board}/${t.task}`)?.at, force));
  await Promise.all(tasks.map(loadAideTaskAi));
  // Fresh is not enough when the PR was read for a part that wanted less (a `pr` part reads the record only): what this one needs is read.
  const prs = [...aideWanted().prs].filter(([id, what]) => !aideIsFresh(aide.prs.get(id)?.at, force) || !aideHasWanted(aide.prs.get(id), what));
  await Promise.all(prs.map(([id, what]) => loadAidePr(id, what)));
}

async function loadAideTaskAi({ board, task }) {
  const key = `${board}/${task}`;
  const n = aide.taskSeq[key] = (aide.taskSeq[key] || 0) + 1;
  // As the boards' and PRs' reads are: one sent before a newer one was applied must not replace it.
  const current = () => {
    if (n < (aide.taskApplied[key] || 0)) return false;
    aide.taskApplied[key] = n;
    return true;
  };
  try {
    const doc = await boardApi(`/b/${board}`, `/api/tasks/${encodeURIComponent(task)}/ai`);
    if (!current()) return;
    aide.taskAis.set(key, { pr: doc.pr || null, record: doc.record || null, at: Date.now(), error: null });
  } catch (e) {
    if (!current()) return;
    // What was read stays: a failed read is not an answer that there is no PR.
    aide.taskAis.set(key, { ...(aide.taskAis.get(key) || { pr: null, record: null }), at: Date.now(), error: e.message });
  }
  renderAide();
}

/* One PR's record, then its read-through when it is wanted and there is one (again only when the record says it changed), then
   its stored diff when that is wanted (again only when the read changed: it can be 8 MB, and it is parsed once). A 404 is "there
   is none", which is not an error. */
async function loadAidePr(id, what) {
  const n = aide.prSeq[id] = (aide.prSeq[id] || 0) + 1;
  const current = () => {
    if (n < (aide.prApplied[id] || 0)) return false;
    aide.prApplied[id] = n;
    return true;
  };
  const e = aide.prs.get(id) || { rec: null, read: undefined, diff: { state: 'loading', files: [] }, resolved: null, settled: false, error: null, at: null, readKey: null, diffKey: null };
  const enc = encodeURIComponent(id);
  const get = async path => {
    try { return await boardApi('', path); } catch (err) { if (err.status === 404) return null; throw err; }
  };
  try {
    const rec = await get(`/api/others/record?id=${enc}`);
    const key = rec ? prReadKey(rec) : 'no-record';
    let { read, diff, readKey, diffKey } = e;
    // A PR with a record that has no read-through has none to ask for.
    if (what.read && (readKey !== key || read === undefined)) {
      read = !rec || rec.ai?.sha ? await get(`/api/others/read?id=${enc}`) : null;
      readKey = key;
    }
    if (what.diff && read && (diffKey !== key || diff.state === 'loading')) {
      const res = await get(`/api/others/diff?id=${enc}`);
      // A diff is what the read-through was made from, or it is not what its points mean.
      diff = res && res.diff != null && res.sha === read.sha ? { state: 'ok', files: prParseDiff(res.diff) } : { state: 'gone', files: [] };
      diffKey = key;
    } else if (what.diff && !read) diff = { state: 'gone', files: [] };
    if (!current()) return;
    Object.assign(e, { rec, read, diff, readKey, diffKey, settled: true, error: null, at: Date.now(),
      resolved: diff.state === 'ok' && read?.annotations?.length ? prDiffResolve(diff.files, read) : null });
  } catch (err) {
    if (!current()) return;
    Object.assign(e, { settled: true, error: err.message, at: Date.now() });
  }
  aide.prs.set(id, e);
  renderAide();
}

/* What was read of a board before now is older than what this page just did to it (an answer): the answers still on their way are dropped. */
const aideBoardWrote = slug => { aide.boardApplied[slug] = (aide.boardSeq[slug] || 0) + 1; };

const aideParentTitle = (nwo, key) => workRepos().find(r => r.nwo === nwo)?.parents?.find(p => p.key === key)?.title || '';

/* The read-through of the PR a task is of, when it has one that has been read: what annotates the lines of a gate's 「根拠」. */
function aideTaskRead(board, taskId) {
  const ai = taskId ? aide.taskAis.get(`${board}/${taskId}`) : null;
  return ai?.pr && ai.record?.ai?.sha ? aide.prs.get(ai.pr)?.read || null : null;
}

/* What `ctx.gate(ref)` says of a gate (see the gate renderer in aide-display.js): null while its board has not been read. */
function aideGateView(ref, rowKeyOf) {
  const cut = ref.lastIndexOf('/');
  const board = ref.slice(0, cut);
  const gateId = ref.slice(cut + 1);
  const entry = [...(work.entries?.values() || [])].find(e => (e.gates || []).some(g => g.key === ref)) || null;
  const known = { board, nwo: entry?.nwo || '', entry, rowKey: entry ? rowKeyOf(entry.id) : null,
    title: entry?.task?.title || entry?.gateTitle || aide.gateTitles.get(ref) || '' };
  const b = aide.boards[board];
  if (!b || (!b.data && !b.error)) return null;
  if (!b.data) return { ...known, error: b.error };
  const g = (b.data.gates || []).find(x => x.id === gateId);
  if (!g || answeredGates.has(ref)) return { ...known, gone: true };
  if (g.title) aide.gateTitles.set(ref, g.title);
  const task = (b.data.tasks || []).find(t => t.id === g.task) || null;
  const m = entry && !entry.isHub && work.doc ? workAwayModel(entry, workMarks(entry.nwo)[entry.id], [], workServerNow()) : null;
  const ai = g.task ? aide.taskAis.get(`${board}/${g.task}`) : null;
  const prEntry = ai?.pr && ai.record?.ai?.sha ? aide.prs.get(ai.pr) : null;
  return { ...known, g, task, refreshError: b.error || ai?.error || prEntry?.error || null, read: aideTaskRead(board, g.task), away: m ? workAwayHtml(m, workServerNow(), false) : '',
    parentTitle: entry?.task?.parent ? aideParentTitle(entry.nwo, entry.task.parent) || entry.task.parent : '' };
}

/* The plan of a task in a line's worth of facts: the plan it works to (approved last), else the one waiting on the person, else
   the newest the task has. */
function aidePlanOf(task, data) {
  const open = (data.gates || []).find(g => g.task === task.id && g.kind === 'plan' && g.wait !== false);
  const newest = [...(task.records || [])].filter(g => g.kind === 'plan').sort((a, b) => (a.openedAt || '').localeCompare(b.openedAt || '')).pop();
  const plan = open ? { g: open, state: 'waiting' } : task.approvedPlan ? { g: task.approvedPlan, state: 'decided' } : newest ? { g: newest, state: 'decided' } : null;
  return plan && { title: plan.g.title, state: plan.state, at: plan.g.answeredAt || plan.g.openedAt, text: plan.g.goal || plan.g.focus || '' };
}

/* Why the last read of a board failed while what was read before is still held, else null. */
const aideBoardError = board => aide.boards[board]?.error || null;

const aideNwoOf = board => boards.find(b => b.slug === board)?.nwo || '';

/* What `ctx.task(board, id)` says: the task of a board the part names, its plan and gates and what comes next. */
function aideTaskView(board, id, rowKeyOf) {
  const b = aide.boards[board];
  if (!b || (!b.data && !b.error)) return null;
  if (!b.data) return { error: b.error, nwo: aideNwoOf(board) };
  const task = (b.data.tasks || []).find(t => t.id === id);
  if (!task) return { gone: true, nwo: aideNwoOf(board) };
  const gates = (b.data.gates || []).filter(g => g.task === id && !answeredGates.has(`${board}/${g.id}`));
  const nwo = aideNwoOf(board);
  const raw = `${nwo}/${board}`;
  const row = work.doc ? workListRows().rows.find(r => r.raw === raw && r.task?.id === id) : null;
  return { task, plan: aidePlanOf(task, b.data), gates, nwo, next: aideTaskNext(task, gates), rowKey: row ? row.key : rowKeyOf(`${board}/${id}`), refreshError: b.error };
}

/* A parent with the progress of its children: what the work document lists of it, joined to the open waits and the tasks it knows. */
function aideParentView(board, key) {
  if (!work.doc) return null;
  const nwo = aideNwoOf(board);
  const repo = workRepos().find(r => r.nwo === nwo && (r.parents || []).some(p => p.key === key)) || workRepos().find(r => (r.parents || []).some(p => p.key === key && p.hub === board));
  const p = repo?.parents?.find(x => x.key === key);
  if (!p) return { gone: true, nwo };
  const waits = aideCurrentWaits().filter(w => w.nwo === repo.nwo && w.parent === key);
  const tasks = new Map();
  for (const r of repo.rows || []) if (r.task) tasks.set(`${r.board}/${r.task.id}`, r.task);
  for (const t of repo.turns || []) if (t.task) tasks.set(`${t.board}/${t.task.id}`, t.task);
  const waitsBy = new Map();
  for (const c of p.children || []) waitsBy.set(aideChildKey(c), waits.filter(w => w.board === c.hub && w.taskId === c.id));
  const listed = workListRows().rows;
  const rowsBy = new Map();
  for (const c of p.children || []) { const r = listed.find(x => x.raw === `${repo.nwo}/${c.hub}` && x.task?.id === c.id); if (r) rowsBy.set(aideChildKey(c), r.key); }
  const { items, counts, total } = aideParentBuckets(p.children || [], waitsBy, tasks, rowsBy, p.total);
  return { p, children: items, counts, total, next: aideParentNext(items, waits), nwo: repo.nwo };
}

/* What `ctx.session(board, id)` says: the row of the list the session is, or one made of what the document lists when the list leaves it out. */
function aideSessionView(board, id) {
  if (!work.doc) return null;
  const repo = workRepos().find(r => (r.rows || []).some(x => x.board === board && x.session.id === id) || (r.hubSessions || []).some(x => x.board === board && x.session.id === id));
  if (!repo) return { gone: true };
  const data = { now: work.doc.now, repo: repo.nwo, hubs: repo.hubs || [], sessions: [] };
  let r = workListRows().rows.find(x => x.raw === `${repo.nwo}/${board}` && x.s.id === id);
  if (!r) {
    const x = (repo.rows || []).find(y => y.board === board && y.session.id === id) || (repo.hubSessions || []).find(y => y.board === board && y.session.id === id);
    r = { s: x.session, st: sessionState(x.session, data), data, task: x.task || null, key: null, nwo: repo.nwo };
  }
  const fns = {
    glyph: st => workGlyphHtml(st),
    label: st => WORK_GLYPH[st]?.[1] || st,
    why: x => unknownWhy(x.s) || agentText(x.s, x.data) || '',
    request: requestText,
    said: m => mdPlain(String(m)).split('\n').map(l => l.trim()).find(Boolean) || '',
    phaseMinutes: x => (x.s.phaseAt != null && x.data.now != null ? minutesSince(x.s.phaseAt, x.data.now) : null),
    title: x => sessionLabel(x.s, false, x.data).text,
  };
  return { r, fns, nwo: repo.nwo, stuckAfter: Number(aide.boards[board]?.data?.stuckAfterMinutes) || 0, refreshError: aideBoardError(board) };
}

/* `ctx.pr(id)`: the record, read-through and parsed diff of a PR the parts name. */
function aidePrView(id) {
  const e = aide.prs.get(id);
  if (!e || !e.settled) return { state: 'loading' };
  // A read that failed is no answer that there is none; one that failed after an earlier success keeps that content, said to be old.
  if (e.error && !e.read) return { state: 'error', error: e.error };
  if (!e.read) return { state: 'none', rec: e.rec };
  return { state: 'ok', rec: e.rec, read: e.read, diff: { ...e.diff, resolved: e.resolved }, refreshError: e.error };
}

/* `ctx.prFacts(ref)`: what the PR part needs, of a task's PR (by the task) or of a PR by its id. The task card is the board's own read of the
   PR (its last refresh), the record the resident's: neither asks GitHub now. */
function aidePrFacts(ref) {
  const nowSecs = workServerNow();
  const cardOf = (board, id) => (aide.boards[board]?.data?.tasks || []).find(t => t.id === id) || null;
  const phaseOf = (board, id) => (workRepos().flatMap(r => r.rows || []).find(r => r.board === board && r.task?.id === id)?.session || {}).phase || null;
  if (ref.pr) {
    const e = aide.prs.get(ref.pr);
    if (!e || !e.settled) return { loading: true };
    if (e.error && !e.rec) return { error: e.error };
    const link = e.rec?.task;
    const card = link ? cardOf(link.board, link.id) : null;
    return { rec: e.rec, task: card, prId: ref.pr, prStatus: card?.prStatus || null, prTurn: card?.prTurn || null, phase: card && link ? phaseOf(link.board, link.id) : null, nowSecs,
      refreshError: e.error || (link ? aideBoardError(link.board) : null) };
  }
  const b = aide.boards[ref.board];
  if (!b || (!b.data && !b.error)) return { loading: true };
  if (!b.data) return { error: b.error };
  const card = cardOf(ref.board, ref.id);
  if (!card) return { error: 'このタスクは、いまのボードにありません' };
  const ai = aide.taskAis.get(`${ref.board}/${ref.id}`);
  if (!ai) return { loading: true };
  const e = ai.pr ? aide.prs.get(ai.pr) : null;
  return { rec: ai.record || e?.rec || null, task: card, prId: ai.pr, prStatus: card.prStatus || null, prTurn: card.prTurn || null, phase: phaseOf(ref.board, ref.id), nowSecs,
    refreshError: ai.error || e?.error || b.error || null };
}

/* `ctx.diffOf(ref)`: the lines' file in the source the part names. A PR's diff is the one stored with its read-through, with the reader's
   annotations anchored in it; a gate's is its own, open only while the gate waits. */
function aideDiffView(ref) {
  if (ref.pr) {
    const e = aide.prs.get(ref.pr);
    if (!e || !e.settled) return { state: 'loading' };
    if (e.error && !e.read) return { state: 'error', error: e.error };
    if (!e.read || e.diff.state === 'gone') return { state: 'gone' };
    if (e.diff.state !== 'ok') return { state: 'loading' };
    const file = e.diff.files.find(f => f.path === ref.path) || null;
    return { state: 'ok', file, res: file && e.resolved ? e.resolved.files.get(file.path) : null, read: e.read, board: e.rec?.task?.board || '', taskId: e.rec?.task?.id || '', refreshError: e.error };
  }
  const gateRef = `${ref.board}/${ref.id}`;
  const b = aide.boards[ref.board];
  if (!b || (!b.data && !b.error)) return { state: 'loading' };
  if (!b.data) return { state: 'error', error: b.error };
  const g = (b.data.gates || []).find(x => x.id === ref.id);
  if (!g || g.wait === false || answeredGates.has(gateRef) || !g.diff) return { state: 'gone' };
  let parsed = aide.gateDiffs.get(gateRef);
  if (!parsed || parsed.diff !== g.diff) aide.gateDiffs.set(gateRef, parsed = { diff: g.diff, files: prParseDiff(g.diff) });
  return { state: 'ok', file: parsed.files.find(f => f.path === ref.path) || null, res: null, read: null, board: ref.board, taskId: g.task || '', gateRef, refreshError: b.error };
}

function aideCtx() {
  const waits = aideCurrentWaits();
  // What each wait is called, kept for when its gate is gone.
  for (const w of waits) if (!aide.gateTitles.has(w.key)) aide.gateTitles.set(w.key, w.title);
  const listed = work.doc ? workListRows() : { rows: [], turns: [], judged: new Map() };
  const rowKeyOf = id => [...listed.rows, ...listed.turns].find(r => r.id === id)?.key || null;
  return {
    waits,
    finished: aideFinished(listed.judged, id => { const r = [...listed.rows, ...listed.turns].find(x => x.id === id); return r ? aideRowTitle(r) : ''; }),
    gate: ref => aideGateView(ref, rowKeyOf),
    task: (board, id) => aideTaskView(board, id, rowKeyOf),
    parent: aideParentView,
    pr: aidePrView,
    prFacts: aidePrFacts,
    session: aideSessionView,
    diffOf: aideDiffView,
    rowKey: rowKeyOf,
    draft: ref => aide.drafts.get(ref) || '',
    parentTitle: aideParentTitle,
    repoName: workRepoName,
    // Not read yet, failed and a gate with no focus are three different things to say.
    focusOf: w => {
      const b = aide.boards[w.board];
      if (!b || (!b.data && !b.error)) return { state: 'loading' };
      if (!b.data) return { state: 'error', error: b.error };
      const g = (b.data.gates || []).find(x => x.id === w.gateId);
      return g ? { state: 'ok', focus: g.focus || '' } : { state: 'gone' };
    },
  };
}

/* ── reading and starting ── */

async function refreshAide(force = false) {
  if (!multiBoard) return;
  if (aide.busy) {
    // An answer to something just done must not be lost to the round already out.
    if (force) aide.again = true;
    return;
  }
  aide.busy = true;
  try {
    const n = ++aide.reads;
    aide.view = await boardApi('', '/api/aide');
    aide.readOk = n;
    aide.error = null;
    try {
      aide.display = aideDisplayOf(await loadAideDisplay());
    } catch {
      // The area keeps what it last showed. A display that cannot be read says nothing about the aide, so it is not the pane's error.
    }
  } catch (e) {
    // The last read stays on screen, with the reason it is old.
    aide.error = e.message;
  } finally {
    aide.busy = false;
  }
  const v = aide.view;
  if (v && aide.wantOpen) {
    // Never on `unknown`: a second aide must not start over one that cannot be told apart from a live one.
    if ((v.state === 'none' || v.state === 'stopped') && v.startable) openAide();
    else aide.wantOpen = false;
  }
  if (v && aide.pendingAsk && !v.startable && v.state !== 'running' && v.state !== 'starting') {
    aide.pendingAsk = null;
    note('副官に聞く', true, AIDE_NOT_STARTABLE);
  }
  renderAide();
  aideFlushAsk();
  // The boards of the parts are read on their own: a slow or failed read of one holds nothing else (not this round, nor the
  // aide's state), and each draws as it arrives. Their order is kept by the per-slug sequence, not by `aide.busy`.
  loadAideBoards(force);
  if (aide.again) {
    aide.again = false;
    refreshAide(true);
  }
}

async function openAide() {
  if (aide.opening) return;
  aide.opening = true;
  aide.wantOpen = false;
  try {
    await boardApi('', '/api/aide/open', { method: 'POST', body: '{}' });
  } catch (e) {
    aide.pendingAsk = null;
    note('副官を起動できません', true, e.status === 409 ? AIDE_NOT_STARTABLE : e.message);
  } finally {
    aide.opening = false;
  }
  refreshAide(true);
}

function resetAide() {
  if (aide.resetting) return;
  aide.resetting = true;
  renderAide();
  boardApi('', '/api/aide/reset', { method: 'POST', body: '{}' }).then(r => {
    // The old socket is of the session that was just stopped; the new one is mounted when a read says it runs.
    disposeTermSlot(aideTerm);
    note('副官をリセットしました', false, r.started === false ? '止めましたが、新しい副官は起動できませんでした' : r.wasBusy ? '作業の途中でしたが止めました' : '新しいセッションで起動しています');
  }, e => {
    note('副官をリセットできません', true, e.status === 409 ? AIDE_NOT_STARTABLE : e.message);
  }).finally(() => {
    aide.resetting = false;
    refreshAide(true);
  });
}

/* ── the terminal ── */

/* Mounted only while the pane is shown and the aide runs: a hidden host has no size to fit to, and a terminal for a session that
   is not there yet would only be told 4404. One already mounted is kept while a row is open, and is the one the way back shows. */
function syncAideTerm(visible) {
  const v = aide.view;
  // A read that could not tell is no reason to drop a socket that works.
  const subject = !v || v.state === 'unknown' ? aideTerm.taskId : v.state === 'running' ? v.sessionId : null;
  if (aideTerm.taskId !== subject || (aideTerm.reconnect && subject)) disposeTermSlot(aideTerm);
  aideTerm.taskId = subject;
  if (!subject || aideTerm.term || !visible) return;
  aideTerm.sessionId = subject;
  aideTerm.ended = null;
  aideTerm.endReason = null;
  const handle = mountSessionTerminal(aideTerm.host(), {
    path: `/api/aide/terminal?session=${encodeURIComponent(subject)}`,
    base: '',
    focus: aideTerm.focus,
    onEnd: (code, reason) => {
      if (aideTerm.term !== handle) return;
      aideTerm.ended = code;
      aideTerm.endReason = reason;
      // 4410: it ended or another took its place; the read says which, and a new session is mounted by the next draw.
      if (code === 4410) refreshAide(true);
      renderAide();
    },
  });
  aideTerm.term = handle;
  aideTerm.focus = false;
}

/* What is under the terminal while there is none, or what it still has to say. */
function aidePlaceholderHtml() {
  const v = aide.view;
  const muted = text => `<div class="tp-muted">${esc(text)}</div>`;
  const start = '<button type="button" class="btn-m3-primary" data-aide-start>副官を起動</button>';
  const lines = [];
  if (aideTerm.term) {
    if (aideTerm.ended != null) {
      lines.push(muted(`副官のターミナルが閉じました: ${terminalEndText(aideTerm.ended, aideTerm.endReason)}`),
        '<button type="button" class="btn-m3-tonal" data-aide-reconnect>再接続</button>');
    }
  } else if (!v) lines.push(muted(aide.error ? `副官の状態を読めませんでした: ${aide.error}` : '読み込み中…'));
  else if (v.state === 'starting') lines.push(muted('副官を起動しています…'));
  else if (v.state === 'running') lines.push(muted('ターミナルに接続しています…'));
  else if (v.state === 'unknown') lines.push(muted(`副官の状態を読めません: ${v.error || '理由は不明です'}`));
  else if (!v.startable) lines.push(muted(AIDE_NOT_STARTABLE));
  else {
    lines.push(muted(v.state === 'stopped' ? '副官は止まっています' : '副官はまだ起動していません'), start,
      muted('最初の起動では、Claude Code がこのフォルダを信頼するかをここで尋ねます'));
  }
  if (v && aide.error) lines.push(muted(`副官の状態を読めませんでした: ${aide.error}`));
  if (aide.pendingAsk) {
    lines.push(muted(`副官の手が空いたら、次を送ります: 「${aide.pendingAsk}」`),
      '<button type="button" class="btn-m3-text" data-aide-ask-cancel>取り消す</button>');
  }
  return lines.join('');
}

/* 「対象」: the topic on display, and a chip for each repository, which puts `[owner/repo] ` at the start of what is typed. */
function aideScopeHtml() {
  const label = aideShown().topic?.label || 'いまの話題なし';
  const repos = workRepos().filter(r => !r.error).slice().sort((a, b) => a.nwo.localeCompare(b.nwo, undefined, { sensitivity: 'base' }));
  const chip = (scope, text, title) => `<button type="button" class="aide-chip" data-aide-scope="${esc(scope)}" title="${esc(title)}">${esc(text)}</button>`;
  return `<span class="aide-scope-label">${aideIcon('my_location')}対象</span>`
    + `<span class="aide-scope-cur" title="何も付けずに聞くと、いま表示領域に出ている話題の続きとして扱います">${esc(label)}</span>`
    + repos.map(r => chip(r.nwo, workRepoName(r.nwo), `[${r.nwo}] を入れて、このリポジトリの話として聞く`)).join('')
    + chip('#', '#…', 'タスク番号を入れる');
}

/* ── drawing ── */

function renderAide() {
  if (view !== 'work') return;
  const on = !nav.task;
  wk('aide').hidden = !on;
  wk('wk-term').classList.toggle('aide-on', on);
  if (on) {
    setAidePart('head', wk('aide-head'), aideHeadHtml(aide.view, { resetting: aide.resetting }));
    syncAideTerm(true);
    const ph = wk('aide-term-ph');
    setAidePart('ph', ph, aidePlaceholderHtml());
    ph.hidden = !aideDrawn.ph;
    setAidePart('scope', wk('aide-scope'), aideScopeHtml());
    // The aide showing something new takes the floor from a part the page made.
    if (aide.local && !aideLocalHolds(aide.display, aide.local)) aide.local = null;
    setAideDisplay(wk('aide-display'), aideDisplayHtml(aideShown(), { news: aideNews(), ctx: aideCtx() }));
    // What the person has now seen: a wait that comes after this is what the count says grew.
    if (work.doc) aide.seen = new Set(aideCurrentWaits().map(w => w.key));
  }
  renderAidePin();
  renderAideBack();
}

/* The row at the top of the list. Only its words and marks change, so it keeps the keyboard focus across a poll. */
function renderAidePin() {
  const pin = wk('aide-pin');
  if (!pin) return;
  const waits = work.doc ? aideCurrentWaits() : null;
  const sub = waits ? aidePinSub(waits) : '読み込み中…';
  const subEl = wk('aide-pin-sub');
  if (subEl.textContent !== sub) subEl.textContent = sub;
  const n = waits ? waits.length : 0;
  const count = wk('aide-pin-count');
  if (count.textContent !== String(n)) count.textContent = String(n);
  count.classList.toggle('zero', n === 0);
  const current = view === 'work' && !nav.task;
  if (current) pin.setAttribute('aria-current', 'true'); else pin.removeAttribute('aria-current');
}

/* Over a row the aide sent the person to: the way back. Over any row: how many waits came while they were away from the aide. */
function renderAideBack() {
  const on = !nav.task;
  const here = !on && !!aide.from && aide.from.board === nav.board && aide.from.ref === nav.task;
  // A row chosen by hand is not one the aide sent the person to.
  if (!here) aide.from = null;
  const back = wk('aide-back');
  setAidePart('back', back, here ? `${aideIcon('support_agent')}<span>副官から開きました — ${esc(aide.from.title)}</span><button type="button" class="aide-back-btn" data-aide-back>${aideIcon('arrow_back')}<span>副官に戻る</span></button>` : '');
  back.hidden = !here;
  const n = !on && work.doc ? aidePeekCount(aide.seen, aideCurrentWaits()) : 0;
  const peek = wk('aide-peek');
  setAidePart('peek', peek, n ? `${aideIcon('support_agent')}<span>副官 · 待ちが ${n} 件増えました</span>` : '');
  peek.hidden = !n;
}

/* ── going to the aide, and asking it ── */

/* The pane in the middle, whatever was selected. `start` is the person asking for it to run; `focus` gives its terminal the keyboard. */
function aideShow({ start = false, focus = true } = {}) {
  if (nav.task) go({ board: 'all', view: 'work', task: null, pane: 'detail' });
  aideTerm.focus = focus;
  if (start) aide.wantOpen = true;
  renderAide();
  refreshAide(true);
  if (focus && aideTerm.term) aideTerm.term.focus();
}

/* Typed into the aide's prompt, and sent, once it can take it. */
function aideAsk(text) {
  const v = aide.view;
  if (v && !v.startable && v.state !== 'running' && v.state !== 'starting') return note('副官に聞く', true, AIDE_NOT_STARTABLE);
  aide.pendingAsk = aideCleanText(text);
  aide.askAfter = aide.reads;
  // Not typed from the last poll's state: the aide may have stopped on a question since. aideShow reads it again.
  aideShow({ start: true });
}

/* Only once the agent has been heard from: its hooks run after the folder-trust question, so an Enter queued before that
   could never answer it. Nor while it waits on the person (a permission or a question): the Enter would pick an answer. */
function aideFlushAsk() {
  const v = aide.view;
  const handle = aideTerm.term;
  if (!aide.pendingAsk || aide.readOk <= aide.askAfter || v?.state !== 'running' || !v.agentSession || v.agentSession.status === 'waiting' || !handle || aideTerm.ended != null) return;
  if (!handle.type(aide.pendingAsk)) return;
  aide.pendingAsk = null;
  // Enter in a chunk of its own: the prompt takes text that arrives with a line break as a paste and keeps the break in it.
  setTimeout(() => { if (aideTerm.term === handle) handle.type('\r'); }, 80);
  handle.focus();
  renderAide();
}

/* A chip pressed just after the terminal mounted finds its socket still connecting: the text waits for it (a few seconds at
   most) rather than being dropped. */
function aideTypeScope(text, tries = 15) {
  const handle = aideTerm.term;
  if (!handle || aideTerm.ended != null) return note('対象', true, '副官のターミナルが開いていません');
  if (handle.type(text)) return handle.focus();
  if (tries <= 0) return note('対象', true, '副官のターミナルにつながりませんでした');
  setTimeout(() => { if (aideTerm.term === handle) aideTypeScope(text, tries - 1); }, 200);
}

/* A row of the list, opened from the aide, with the way back. The one entry for what shows a session (「ターミナルを見る」). */
function aideOpenRow(key) {
  const { rows, turns } = workListRows();
  const r = [...rows, ...turns].find(x => x.key === key) || work.hubsByKey?.get(key);
  if (!r) return note('ターミナルを見る', true, 'その仕事は、いまの一覧にありません');
  aide.from = { board: r.board, ref: r.ref, title: aideRowTitle(r) };
  selectWorkRow(r);
}

/* ── events ── */

wk('aide-pin').addEventListener('click', () => aideShow({ start: true }));

wk('wk-term').addEventListener('click', e => {
  let b;
  if (e.target.closest('[data-aide-back], #aide-peek')) return aideShow();
  if (e.target.closest('[data-aide-start]')) return aideShow({ start: true });
  if (e.target.closest('[data-aide-reconnect]')) {
    aideTerm.reconnect = true;
    aideTerm.focus = true;
    return renderAide();
  }
  if (e.target.closest('[data-aide-ask-cancel]')) {
    aide.pendingAsk = null;
    return renderAide();
  }
  if ((b = e.target.closest('[data-aide-scope]'))) {
    const scope = b.dataset.aideScope;
    return aideTypeScope(scope === '#' ? '#' : aideScopeText(scope));
  }
  if ((b = e.target.closest('[data-aide-row]'))) return aideOpenRow(b.dataset.aideRow);
  if (e.target.closest('[data-aide-ask-new]')) {
    const news = aideNews();
    if (!news) return;
    return aideAsk(news.count === 1 ? aideAskRowText(news.first) : AIDE_ASK_ALL);
  }
  if (e.target.closest('[data-aide-reset]')) {
    if (aide.resetting) return;
    // A reset in the middle of a turn throws that turn away: asked first.
    if (aideStateOf(aide.view).busy) {
      const dialog = wk('aide-reset-dialog');
      dialog.returnValue = '';
      return dialog.showModal();
    }
    resetAide();
  }
});

/* ── events of the parts ── */

/* The part the page puts on top: `part` over the aide's parts, until the aide shows something new. */
function aideSetLocal(part, topic) {
  aide.local = { part, serverTop: aide.display.parts[0]?.id ?? null, topic };
  renderAide();
}

function aideBackToWaits() {
  aideSetLocal(aideWaitsPart(), AIDE_WAITS_TOPIC);
  refreshAide(true);
}

function aideOpenGate(key) {
  const cut = key.lastIndexOf('/');
  if (cut <= 0) return;
  const w = aideCurrentWaits().find(x => x.key === key);
  aideSetLocal(aideGatePart(key.slice(0, cut), key.slice(cut + 1)), { key: 'page', label: `${w ? workRepoName(w.nwo) : ''} · 判断待ち` });
  // The board is read now: what the part shows must not be a poll old.
  refreshAide(true);
}

/* After an answer: the next wait of the same parent when 「処理したら次へ」 is on and there is one, else the list of waits.
   The aide is not told and nothing is written to the server. `done` is the wait as it was before the answer took it off the list. */
function aideAfterAnswer(done, top) {
  // The person may have opened another part while the answer was on its way: that is what they are on, so it stays.
  if (aideShown().parts[0]?.id === top) {
    const next = prefs.reviewNext ? aideNextWait(aideCurrentWaits(), done) : null;
    if (next) aideSetLocal(aideGatePart(next.board, next.gateId), { key: 'page', label: `${workRepoName(next.nwo)} · 次の判断待ち` });
    else aideSetLocal(aideWaitsPart(), AIDE_WAITS_TOPIC);
  }
  // The work document, the aide's read (the boards of the parts with it) and the sidebar, so that the answer is off the list and the next gate is read now.
  refresh(true);
  refreshBoards();
}

async function aideDecide(b) {
  const box = b.closest('[data-aide-gate]');
  const ref = box.dataset.aideGate;
  if (deciding.has(ref)) return;
  const view = aideGateView(ref, () => null);
  const g = view?.g;
  if (!g || g.wait === false) return note('判断', true, 'この判断待ちは、いま読めません。もう待っていないか、読み込みに失敗しています');
  const act = b.matches('.pick[data-choice]') ? 'choice' : b.dataset.act;
  const typed = box.querySelector('.gate-comment')?.value.trim() || '';
  // Taken before the answer: the answer takes the gate off the list the next one is looked for in.
  const wait = aideCurrentWaits().find(w => w.key === ref);
  const done = wait || { key: ref, nwo: view.nwo, parent: null, taskId: g.task || null };
  const top = aideShown().parts[0]?.id;
  deciding.add(ref);
  let sent;
  try {
    sent = act === 'close' ? await closeGateOf(g, typed) : await answerGate(g, act, act === 'choice' ? b.dataset.choice : undefined, typed);
  } finally {
    deciding.delete(ref);
  }
  // A refused answer keeps what was typed.
  if (!sent) return;
  aide.drafts.delete(ref);
  aideBoardWrote(view.board);
  aideAfterAnswer(done, top);
}

function aidePark(board, taskId) {
  const task = aide.boards[board]?.data?.tasks?.find(t => t.id === taskId);
  if (task) openParkDialog(task, task._base);
  else note('置く', true, 'このタスクは、いま読めません');
}

/* The read-through of a PR, in the view that has it: a task's PR in its 「AI 下読み」 tab, a review request in its own panel. The way back
   is the aide's, as for a row. 「読み手に聞く」 goes to the same place, where the reader's terminal is, and types nothing. */
function aideOpenRead(d) {
  const id = d.aidePr;
  if (!id) return;
  const rec = othersById(id);
  if (d.aideBoard && d.aideTask) {
    aide.from = { board: d.aideBoard, ref: d.aideTask, title: rec ? `AI 下読み — ${prSubText(rec)}` : `AI 下読み — ${id}` };
    return go({ board: d.aideBoard, view: 'work', task: d.aideTask, pane: 'ai' });
  }
  aide.from = { board: 'all', ref: PR_REF + id, title: rec ? `AI 下読み — ${prSubText(rec)}` : `AI 下読み — ${id}` };
  if (rec) return prOpen(rec);
  go({ board: 'all', view: 'work', task: PR_REF + id, pane: 'detail' });
}

wk('aide-display').addEventListener('click', e => {
  let b;
  if ((b = e.target.closest('[data-aide-open-read], [data-aide-ask-reader]:not([disabled])'))) return aideOpenRead(b.dataset);
  if ((b = e.target.closest('[data-aide-open-gate]'))) return aideOpenGate(b.dataset.aideOpenGate);
  if (e.target.closest('[data-aide-waits]')) return aideBackToWaits();
  if ((b = e.target.closest('[data-aide-park]'))) return aidePark(b.dataset.aideBoard, b.dataset.aideTask);
  if ((b = e.target.closest('[data-aide-expand]'))) {
    // Part of the task's 「拡大」 dialog (card-dialog.js); until it is there the button says so.
    const args = { board: b.dataset.aideBoard, taskId: b.dataset.aideTask, target: b.dataset.aideTarget, gate: b.dataset.aideGateRef || null, opener: b };
    return typeof openTaskDialogFor === 'function' ? openTaskDialogFor(args) : note('拡大で開く', true, 'この画面ではまだ開けません');
  }
  if ((b = e.target.closest('[data-aide-gate] [data-act], [data-aide-gate] .pick[data-choice]'))) aideDecide(b);
});

wk('aide-display').addEventListener('input', e => {
  const box = e.target.closest?.('.gate-comment');
  const ref = box?.closest('[data-aide-gate]')?.dataset.aideGate;
  if (!ref) return;
  if (box.value) aide.drafts.set(ref, box.value); else aide.drafts.delete(ref);
});

wk('aide-display').addEventListener('focusout', () => {
  if (!aide.held) return;
  // Not at once: the focus may be leaving for a button that is being pressed, and a redraw now would replace it under the pointer
  // (a browser that does not focus a pressed button says nothing of where the focus went).
  setTimeout(() => { if (aide.held) renderAide(); }, 300);
});

wk('aide-reset-dialog').addEventListener('close', e => {
  if (e.target.returnValue === 'reset') resetAide();
});
