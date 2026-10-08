/* ── いまの仕事: what is new and what was looked at ─────────────────────────────────────────────
   Pure: these read the work document and the read marks they are handed and return, touching neither the
   page nor local storage, so `src/ui/tests/my-work-seen.test.js` runs them under `node --test`. Loaded
   after util.js (`stampSecs`, `kindOf`) and before my-work.js, which draws and keeps the marks.

   A row holds items, one for each thing it waits on the person for, and each item has a `since`: when it
   began, on the server's clock. A row is read when every item began at or before the later of the time the
   person left it and the time they acted on it. 新着 is a row with an item they have not read; 後で見る a row
   whose items they have all read and which still has one. Each time is a number of seconds, as a mark keeps it:
     left     the person left the row after opening it, or pressed "mark all read"
     back     the person sent it back to 新着 by hand
     cleared  the person cleared it with ✓ (done, failed and PR items only; a gate or a permission wait
              ends only when it is answered) */

const WORK_HUB_REF = 'hub:';
const WORK_SESS_REF = 'session:';
/* The ref of a row that is only a gate, when no hub or session can hold it. */
const WORK_GATE_REF = 'gate:';
/* The PR turns that are the person's to act on, as the server names them. */
const WORK_PR_WORDS = {
  changes: '修正の依頼が残ったまま',
  'ci-failed': 'CI が落ちたまま',
  merge: 'マージ待ち',
  closed: '閉じられたまま',
};
/* Which kinds a ✓ clears: what the person only has to take notice of. */
const WORK_CLEARABLE = ['done', 'failed', 'pr'];
/* The order the words of a 後で見る row name what is open, most pressing first. */
const WORK_ITEM_ORDER = ['gate', 'permission', 'failed', 'pr', 'done'];
/* How long a mark is kept for a row that is no longer listed. */
const WORK_MARK_KEPT_SECS = 7 * 86400;

/* The session of two that a row shows: one that runs over one that is gone, then the one heard from last. */
function workBetterSession(a, b) {
  if (!!a.present !== !!b.present) return !!a.present;
  return (a.agentSession?.updatedAt || a.phaseAt || 0) > (b.agentSession?.updatedAt || b.phaseAt || 0);
}

/* The rows of the work document as entries, one to a row id (`<board>/<ref>`, the board the server says owns
   it): the task, or `hub:<id>`, or `session:<id>`. A task or parent-task hub that waits on the person with no
   session is an entry too, so what has nothing running is not lost. `isGateDone` says of `<slug>/<id>` whether
   the person answered that gate on this page, which the document does not yet know. */
function workEntries(doc, isGateDone = () => false) {
  const out = [];
  for (const repo of doc?.repos || []) {
    const byId = new Map();
    const entry = (board, ref) => {
      const id = `${board}/${ref}`;
      if (!byId.has(id)) byId.set(id, { id, nwo: repo.nwo, board, ref, row: null, task: null, session: null, isHub: false, gates: [], items: [] });
      return byId.get(id);
    };
    // The gates of the repository, once each, whichever of the places below names them first.
    const seen = new Set();
    const addGate = (e, slug, gate, openedAt) => {
      const key = `${slug}/${gate.id}`;
      if (seen.has(key) || isGateDone(key)) return;
      seen.add(key);
      e.gates.push({ kind: 'gate', gate: gate.kind, since: stampSecs(openedAt), key });
    };

    for (const r of repo.rows || []) {
      const task = r.task || null;
      const isHub = r.session.kind === 'hub';
      const e = entry(r.board, task ? task.id : isHub ? WORK_HUB_REF + r.session.id : WORK_SESS_REF + r.session.id);
      // Of several sessions of one task, the one the list shows.
      if (e.session && !workBetterSession(r.session, e.session)) continue;
      Object.assign(e, { row: r, task, session: r.session, isHub });
    }
    for (const h of repo.hubSessions || []) {
      Object.assign(entry(h.board, WORK_HUB_REF + h.session.id), { row: h, session: h.session, isHub: true });
    }
    // Gates and PRs on a task: what the person has to do for a task that may have no session.
    const orphans = [];
    for (const t of repo.turns || []) {
      for (const g of t.gates || []) {
        const id = g.task || t.task?.id;
        // A gate that names no task is the waiting of the session it was opened from, below.
        if (!id) { orphans.push(g); continue; }
        const e = entry(t.board, id);
        if (!e.task && t.task) e.task = t.task;
        addGate(e, g.slug, g, g.openedAt);
      }
      if (t.task) {
        const e = entry(t.board, t.task.id);
        if (!e.task) e.task = t.task;
      }
    }
    for (const e of [...byId.values()]) {
      const w = e.session?.waiting;
      if (w) addGate(e, w.slug, w, w.openedAt);
    }
    // A gate no session waits on (its hub is gone, or its `waiting` names another gate) is the hub's of its board, else its own.
    for (const g of orphans) {
      if (seen.has(`${g.slug}/${g.id}`) || isGateDone(`${g.slug}/${g.id}`)) continue;
      const hub = [...byId.values()].find(e => e.isHub && e.board === g.slug);
      const e = hub || entry(g.slug, `${WORK_GATE_REF}${g.slug}/${g.id}`);
      if (!hub) e.gateTitle = g.title || '';
      addGate(e, g.slug, g, g.openedAt);
    }
    for (const e of byId.values()) {
      e.items = workItems(e);
      out.push(e);
    }
  }
  return out;
}

/* What an entry waits on the person for, with when each began. The ledger's times are the status's own
   (`updatedAt` moves only when the status changes), so a session that goes on waiting is one item. A
   parent-task hub that finishes a turn is not one: it is a hub, and has nothing to clear. */
function workItems(e) {
  const items = [...e.gates];
  const a = e.session?.present ? e.session.agentSession : null;
  if (a && !a.error) {
    const since = a.updatedAt ?? null;
    if (a.status === 'waiting') items.push({ kind: 'permission', since });
    else if (a.status === 'failed') items.push({ kind: 'failed', since });
    else if (a.status === 'done' && !e.isHub && !e.gates.length) items.push({ kind: 'done', since });
  }
  const t = e.task;
  if (t?.waitsOnPerson && WORK_PR_WORDS[t.prTurn]) items.push({ kind: 'pr', turn: t.prTurn, since: stampSecs(t.prTurnAt) });
  return items;
}

/* When the person last acted on the entry other than by leaving it: answered a gate of its task, or typed into
   its session, wherever they did it. -Infinity when never. */
function workActedAt(e) {
  const times = [stampSecs(e.task?.gateAnsweredAt), e.session?.present ? e.session.agentSession?.lastPromptAt : null];
  return Math.max(-Infinity, ...times.filter(x => Number.isFinite(x)));
}

/* The items the person has not cleared. A ✓ clears what began before it. */
function workLiveItems(items, mark = {}) {
  const cleared = Number.isFinite(mark.cleared) ? mark.cleared : null;
  return items.filter(i => !(cleared != null && WORK_CLEARABLE.includes(i.kind) && (i.since == null || i.since <= cleared)));
}

/* 'new', 'later' or null (the entry has nothing on it for the person). */
function workSeenClass(items, mark = {}, actedAt = -Infinity) {
  const live = workLiveItems(items, mark);
  if (!live.length) return null;
  const read = Math.max(Number.isFinite(mark.left) ? mark.left : -Infinity, actedAt);
  if (Number.isFinite(mark.back) && mark.back > read) return 'new';
  // An item with no time (a PR record from before the time was kept) is new until the row has been read once.
  const later = i => i.since == null ? read === -Infinity : i.since > read;
  return live.some(later) ? 'new' : 'later';
}

/* The class of an entry among the marks of its repository (`id` to mark). */
function workClassOf(e, marks) {
  return workSeenClass(e.items, marks?.[e.id] || {}, workActedAt(e));
}

/* What is still open, in the words of a 後で見る row. */
function workLaterText(items) {
  const sorted = [...items].sort((a, b) => WORK_ITEM_ORDER.indexOf(a.kind) - WORK_ITEM_ORDER.indexOf(b.kind));
  const first = sorted[0];
  if (!first) return '';
  const what = first.kind === 'gate' ? `${kindOf(first.gate)[0]}が開いたまま`
    : first.kind === 'permission' ? '許可待ちのまま'
      : first.kind === 'failed' ? '失敗したまま'
        : first.kind === 'pr' ? WORK_PR_WORDS[first.turn] || 'PR が残ったまま'
          : '片付けていない';
  return `既読 · ${what}${sorted.length > 1 ? ` ほか ${sorted.length - 1} 件` : ''}`;
}

/* ── the marks ── */

/* What local storage held, read as marks: anything that is not an object of objects of times is dropped, so
   a damaged or foreign value is no marks and not an error. */
function workParseMarks(text) {
  let raw;
  try { raw = JSON.parse(text); } catch (e) { return {}; }
  const out = {};
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return out;
  for (const [id, m] of Object.entries(raw)) {
    if (id === '__proto__' || !m || typeof m !== 'object' || Array.isArray(m)) continue;
    const mark = {};
    for (const f of ['left', 'back', 'cleared']) if (typeof m[f] === 'number' && Number.isFinite(m[f])) mark[f] = m[f];
    if (Object.keys(mark).length) out[id] = mark;
  }
  return out;
}

/* `marks` with `patch` applied to the row `id`: a time only moves later, so a mark written from another tab,
   or from before, is never undone by an older one. */
function workMarkMerge(marks, id, patch) {
  const cur = marks[id] || {};
  const mark = { ...cur };
  for (const [f, v] of Object.entries(patch)) if (Number.isFinite(v)) mark[f] = Math.max(Number.isFinite(cur[f]) ? cur[f] : -Infinity, v);
  return { ...marks, [id]: mark };
}

/* The marks of rows that are neither listed (`liveIds`) nor touched within a week: what is kept is bounded, and
   a row that drops out of one poll and is back in the next keeps its marks. */
function workPruneMarks(marks, liveIds, now) {
  const out = {};
  for (const [id, m] of Object.entries(marks)) {
    const newest = Math.max(-Infinity, ...['left', 'back', 'cleared'].map(f => m[f]).filter(Number.isFinite));
    if (liveIds.has(id) || newest >= now - WORK_MARK_KEPT_SECS) out[id] = m;
  }
  return out;
}
