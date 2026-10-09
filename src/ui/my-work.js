/* ── いまの仕事: the work under way in every repository, in one list ─────────────────────────────
   Left, the list (`/api/work`, one document for the whole server); middle, the terminal of the
   selected session; right, the body's own task panel, on the board of what was selected. The list
   keeps its own data apart from `state`: `state` is the selected board's, which is what the panel's
   actions and the terminal's socket belong to (`BASE`), and it is empty while nothing is selected.
   The first half is pure (it reads the work document and returns); the second draws. */

const wk = id => document.getElementById(id);

/* The boxes of 「状態」, in order. 新着 holds the rows `workSeenClass` (my-work-seen.js) puts in it; a row not in it is in the
   box its state says. */
const WORK_BOXES = [
  { id: 'new', label: '新着' },
  { id: 'running', label: '実行中' },
  { id: 'other', label: 'そのほか' },
];
/* The states a session is listed in. A worker that is gone (none, ended) is not work under way: a
   PR whose worker is gone is not here. */
const WORK_LISTED = ['waiting', 'permission', 'working', 'idle', 'unknown', 'done', 'failed', 'stopped', 'restarting'];
const WORK_RUNNING = ['working', 'idle', 'unknown', 'restarting'];
/* What a child of a parent is, as the server's `progress` says: the class of its segment and its words. */
const WORK_PROGRESS = {
  merged: ['merged', 'マージ済み'],
  done: ['done', '完了'],
  pr: ['pr', 'PR あり'],
  working: ['working', '作業中'],
  'not-started': ['not-started', '未着手'],
};
/* The icon of a state: Material Symbols name, then the label read aloud. Colours are the stylesheet's (my-work.css). */
const WORK_GLYPH = {
  waiting: ['front_hand', '確認待ち'],
  permission: ['front_hand', '入力待ち'],
  working: ['progress_activity', '作業中'],
  restarting: ['progress_activity', '再起動中'],
  done: ['check_circle', '待機中'],
  failed: ['error', 'エラー'],
  stopped: ['warning', '停止'],
  idle: ['pause_circle', '待機中（出力なし）'],
  unknown: ['visibility_off', '状態不明'],
  seen: ['check_circle', '確認済み'],
  parked: ['schedule', '置いている'],
};
const WORK_SUBAGENTS_SHOWN = 4;

const work = {
  doc: null,
  json: '',           // the document as last drawn (workSig)
  minute: null,
  busy: false,
  again: false,       // a forced round asked for while one was out
  error: null,
  structure: '',      // what the list was last built from (renderWorkList)
  cells: new Map(),   // each row's and header's own html, by key
  rowsByKey: new Map(),
  hubsByKey: new Map(), // the hub chips on the headings, by key (workRepoHubChips)
  open: null,         // the row the person has open: { nwo, id, nav, at } (`at`: the server's time it was opened), the mark `left` is written when it is left (it feeds 離れていた間に only; leaving does not read the row)
  entries: new Map(), // the rows' entries by id (my-work-seen.js), as last drawn
  newOrder: [],       // the ids of the rows of 新着 in the order the list last drew them
};
/* The middle terminal: a second slot beside the panel's. It connects through the
   board that was selected, since `state` is that board's. */
const workTerm = { host: () => wk('wk-term-host'), redraw: () => renderWorkTerm(), base: () => BASE,
  taskId: null, sessionId: null, term: null, ended: null, reconnect: false, board: null,
  // Selecting a row must not take the keyboard from the list: `focusWorkTerm` is the one way in.
  focus: false };

/* ── what the document says ── */

const workRepos = () => work.doc?.repos || [];

/* The board a row opens on: its own when this server has that board, else the repository's carrier, which lists
   what the board would have (the panel then says the task's board is not here). Before the boards are known, the row's own. */
const workBoardOf = (repo, board) => !boards.length || boards.some(b => b.slug === board) ? board : repo.carrier;

/* A parent by its key, with the repository it is in. */
function workParentOf(key) {
  for (const repo of workRepos()) {
    const p = (repo.parents || []).find(x => x.key === key);
    if (p) return { ...p, repo };
  }
  return null;
}

/* The document reduced to what is drawn, for comparing two. The server's clock and the times that move with
   every event are left out, or rounded to the minute, as the page's other polls compare; the rate limits are
   in it, apart from when they were last heard. */
function workSig(doc) {
  const minutes = secs => secs == null ? null : Math.floor(secs / 60);
  const session = ({ lastActivityAt, agentSession: a, ...s }) => ({
    ...s,
    lastActivityAt: minutes(lastActivityAt),
    ...(a ? { agentSession: {
      ...a, updatedAt: minutes(a.updatedAt), lastEventAt: undefined, lastMessageAt: minutes(a.lastMessageAt),
      subagents: (a.subagents || []).map(x => ({ id: x.id, type: x.type, activity: x.activity })),
    } } : {}),
  });
  const rates = (doc.rateLimits?.accounts || []).map(({ lastEventAt, ...a }) => ({
    ...a,
    fiveHour: a.fiveHour && { ...a.fiveHour, resetsAt: minutes(a.fiveHour.resetsAt) },
    sevenDay: a.sevenDay && { ...a.sevenDay, resetsAt: minutes(a.sevenDay.resetsAt) },
  }));
  return {
    rates, ratesError: doc.rateLimits?.error,
    repos: (doc.repos || []).map(r => ({
      ...r,
      rows: (r.rows || []).map(x => ({ ...x, session: session(x.session) })),
      hubSessions: (r.hubSessions || []).map(x => ({ ...x, session: session(x.session) })),
    })),
  };
}

/* The rows of the list: every session the document lists whose state is work under way, each with the
   session as the page reads it (a gate answered here is not waited on again). */
function workRows(doc = work.doc) {
  const out = [];
  for (const repo of doc?.repos || []) {
    const data = { now: doc.now, repo: repo.nwo, hubs: repo.hubs || [], sessions: [] };
    for (const r of repo.rows || []) {
      let s = r.session;
      if (s.waiting && answeredGates.has(`${s.waiting.slug}/${s.waiting.id}`)) s = { ...s, waiting: null };
      const st = sessionState(s, data);
      if (!WORK_LISTED.includes(st)) continue;
      const isHub = s.kind === 'hub';
      const ref0 = r.task ? r.task.id : isHub ? HUB_REF + s.id : SESS_REF + s.id;
      const board = workBoardOf(repo, r.board);
      // A task whose board is not served is read on the carrier, where a bare task id names the
      // repository's own task: the session opens instead.
      const ref = r.task && board !== r.board ? SESS_REF + s.id : ref0;
      out.push({ id: `${r.board}/${ref0}`, key: `${board}/${ref}`, raw: `${repo.nwo}/${r.board}`, board, ref, repo, nwo: repo.nwo, s, st, data, task: r.task || null, isHub });
    }
  }
  // One row to a task: its best session.
  const best = new Map();
  for (const r of out) {
    if (!r.task) continue;
    const k = `${r.raw}/${r.task.id}`;
    if (!best.has(k) || workBetter(r.s, best.get(k).s)) best.set(k, r);
  }
  return out.filter(r => !r.task || best.get(`${r.raw}/${r.task.id}`) === r);
}

/* Which of two sessions of one task is the one to show: a live one over one that is gone, then the one heard from last. */
function workBetter(a, b) {
  if (!!a.present !== !!b.present) return !!a.present;
  return (a.agentSession?.updatedAt || a.phaseAt || 0) > (b.agentSession?.updatedAt || b.phaseAt || 0);
}

/* ── what is new and what was read ── */

/* The marks of one repository, kept in this browser (`adj.seenWork.<nwo>`): row id to { left, read, back, cleared, parked }, in seconds on
   the server's clock (my-work-seen.js says what each means). Read from storage every time, never cached, so that what another
   tab wrote is what is read; a value that is not marks reads as none. Also what the while-away timeline (#556) reads. */
const WORK_SEEN_PREFIX = 'adj.seenWork.';
function workMarks(nwo) {
  try { return workParseMarks(localStorage.getItem(WORK_SEEN_PREFIX + nwo) || ''); } catch { return {}; }
}

/* The server's clock as of the document last drawn: a mark covers what that document showed and nothing that came after. */
const workServerNow = () => work.doc?.now || Date.now() / 1000;

/* Write `patches` ([id, { left | read | back }]) onto the marks of `nwo`. The marks are read again first and each time
   only moves later, so another tab's write is not undone; the ones of rows long gone are dropped, unless the document is
   not a complete reading of the repository (a failed read is not absence). */
function workWriteMarks(nwo, patches) {
  let marks = workMarks(nwo);
  for (const [id, patch] of patches) {
    // Read after a send back, and sent back after a read: each is later than the other's time, whatever the document's clock says.
    // `left` is not coupled: it no longer decides what is new.
    const cur = marks[id] || {};
    const p = { ...patch };
    if (p.read != null && cur.back != null) p.read = Math.max(p.read, cur.back);
    if (p.back != null) p.back = Math.max(p.back, cur.read ?? -Infinity, cur.cleared ?? -Infinity) + 0.001;
    marks = workMarkMerge(marks, id, p);
  }
  const repo = workRepos().find(r => r.nwo === nwo);
  if (work.doc && !work.error && repo && !repo.error) {
    const live = new Set(workEntries(work.doc).filter(e => e.nwo === nwo).map(e => e.id));
    marks = workPruneMarks(marks, live, workServerNow());
  }
  try { localStorage.setItem(WORK_SEEN_PREFIX + nwo, JSON.stringify(marks)); } catch {}
  // The `storage` event is for the other tabs.
  if (view === 'work') renderWorkList();
  renderWorkBadge();
}

/* The entries of the document and the class of each ('new' or none), with the marks they were judged by. */
function workJudge() {
  const entries = workEntries(work.doc, key => answeredGates.has(key));
  const marks = new Map();
  const markOf = nwo => marks.get(nwo) || marks.set(nwo, workMarks(nwo)).get(nwo);
  const out = new Map();
  for (const e of entries) {
    const mark = markOf(e.nwo)[e.id] || {};
    // `live` drops a finished item the person has read, so that a repository with nothing else under way can idle out (#600).
    out.set(e.id, { entry: e, cls: workSeenClass(e.items, mark, workActedAt(e)), live: workLiveItems(e.items, mark, workActedAt(e)),
      // The row open has its away count read as of now: its mark still holds the last visit's `left`, which the panel reads.
      away: workAwayCount(e, mark, work.open?.id === e.id) });
  }
  return out;
}

/* When something last happened to an entry, for ordering: the newest of its items. */
const workNewest = r => Math.max(0, ...(r.live || []).map(i => i.since).filter(Number.isFinite));

/* A row for an entry the list has no session row for: a task that waits on the person with nothing running, or a session
   the list does not show (one that is gone). */
function workTurnRow(e, judged, repo) {
  // The board and ref as `workRows` resolves them: a board this server does not serve is read on the carrier, where the
  // session (if any) is what opens.
  const board = workBoardOf(repo, e.board);
  const ref = board !== e.board && e.session ? SESS_REF + e.session.id : e.ref;
  const gate = e.items.some(i => i.kind === 'gate');
  const parkedOnly = !gate && e.items.every(i => i.kind === 'parked');
  return {
    id: e.id, key: `${board}/${ref}`, raw: `${repo.nwo}/${e.board}`, board, ref, repo, nwo: repo.nwo,
    s: {}, st: gate ? 'waiting' : parkedOnly ? 'parked' : 'done', data: { now: work.doc.now, repo: repo.nwo, hubs: repo.hubs || [], sessions: [] },
    task: e.task, isHub: e.isHub, turn: true, entry: e, cls: judged.cls, live: judged.live, away: judged.away,
  };
}

/* The rows of the list as it is drawn: the session rows, each with its class, and the rows of what waits on the person with no
   session row (the band, 「状態」 and 「親 Issue」 show those too). A read or parked one stays while it has anything live. */
function workListRows() {
  const judged = workJudge();
  const rows = workRows();
  const listed = new Set();
  for (const r of rows) {
    const j = judged.get(r.id);
    listed.add(r.id);
    Object.assign(r, { entry: j?.entry || null, cls: j?.cls || null, live: j?.live || [], away: j?.away || 0 });
  }
  const turns = [];
  for (const [id, j] of judged) {
    const repo = workRepos().find(x => x.nwo === j.entry.nwo);
    if (j.live.length && !listed.has(id) && repo) turns.push(workTurnRow(j.entry, j, repo));
  }
  work.entries = new Map([...judged].map(([id, j]) => [id, j.entry]));
  return { rows, turns, judged };
}

/* 新着 as a list, newest first: each row with what it is in. */
function workBands(rows) {
  const band = (id, label, empty) => {
    const mine = rows.filter(r => r.cls === id).sort((a, b) => workNewest(b) - workNewest(a) || (a.key < b.key ? -1 : 1));
    return { key: `band:${id}`, kind: 'band', band: id, label, empty, readAll: id === 'new', rows: mine, items: mine.map(row => ({ row, place: id })) };
  };
  return [band('new', '新着', '新しく来たものはありません')];
}

/* How many rows are new. */
const workNewCount = () => work.doc ? [...workJudge().values()].filter(j => j.cls === 'new').length : 0;

/* The sidebar's count: how many rows are new, whichever view is on. It is the tab's too. */
function renderWorkBadge() {
  const el = wk('work-new-count');
  if (!el) return;
  const n = workNewCount();
  el.textContent = n;
  el.classList.toggle('zero', !n);
  renderDocTitle();
}

/* The row the person leaves: `left` is written for 離れていた間に; the row is not read by it. Called when the address moves off it, when the view does, and when the
   page goes. */
function workLeave() {
  const open = work.open;
  work.open = null;
  if (open && work.doc) workWriteMarks(open.nwo, [[open.id, { left: workServerNow() }]]);
}

/* The id of the row a selection is, as the entries have it. */
function workSelectedId(sel) {
  const r = sel?.row;
  if (!r) return null;
  const s = r.session;
  return `${r.board}/${r.task ? r.task.id : s ? (s.kind === 'hub' ? HUB_REF : SESS_REF) + s.id : nav.task}`;
}

/* Follow the address: moving it off the open row leaves that one, and arriving on a row (by a click or by a link) opens it. A
   poll that drops the row for a moment is not leaving, as the address has not moved. */
function workTrack() {
  const addr = nav.board && nav.board !== 'all' && nav.task && !isParentRef(nav.task) ? `${nav.board}/${nav.task}` : null;
  if (work.open && work.open.nav !== addr) workLeave();
  if (work.open || !addr) return;
  const sel = workSelected();
  const id = workSelectedId(sel);
  if (id && sel.repo) {
    work.open = { nwo: sel.repo.nwo, id, nav: addr, at: workServerNow() };
    // The row just opened has its own away count read as of now: it goes at once, not at the next poll.
    if (view === 'work') renderWorkList();
  }
}

window.addEventListener('pagehide', workLeave);
// Another tab of this browser marked something: the list and the count follow.
window.addEventListener('storage', e => {
  if (e.key != null && !e.key.startsWith(WORK_SEEN_PREFIX)) return;
  if (view === 'work') renderWorkList();
  renderWorkBadge();
});

const workBoxOf = row => row.cls || (WORK_RUNNING.includes(row.st) ? 'running' : 'other');
const workOwnerName = nwo => { const i = (nwo || '').indexOf('/'); return i > 0 ? nwo.slice(0, i) : ''; };

/* The repository nodes under one owner node each, owners in name order. Owners are grouped without regard to case, as
   repositories are matched elsewhere; a name with no slash goes under 「オーナーなし」, last. */
function workOwnerGroups(repoNodes) {
  const groups = new Map();
  for (const repo of repoNodes) {
    const name = workOwnerName(repo.nwo);
    const key = `owner:${name.toLowerCase()}`;
    if (!groups.has(key)) groups.set(key, { key, kind: 'owner', label: name || 'オーナーなし', rows: [], items: [] });
    const group = groups.get(key);
    group.rows.push(...repo.rows);
    group.items.push(repo);
  }
  const rank = g => g.key === 'owner:' ? 1 : 0;
  return [...groups.values()].sort((a, b) => rank(a) - rank(b) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/* The rows of one 「状態」 box under a heading per repository, `owner/name` in name order with case ignored: by owner as
   the tree orders its owners, then by name. The rows come in sorted and keep that order. The key is apart from the
   tree's (`repo:`, `owner:`), so a fold is kept per box. A hub is a chip on its repository's heading, which is drawn
   even when no row of the box is under it. */
function workBoxRepoGroups(boxId, rows, place, chips = []) {
  const groups = new Map();
  const group = nwo => {
    const key = `brepo:${boxId}/${nwo.toLowerCase()}`;
    if (!groups.has(key)) groups.set(key, { key, kind: 'brepo', label: nwo, rows: [], items: [], hubs: [] });
    return groups.get(key);
  };
  for (const row of rows) {
    const g = group(row.nwo || '');
    g.rows.push(row);
    g.items.push({ row, place });
  }
  for (const chip of chips) group(chip.nwo || '').hubs.push(chip);
  const order = g => { const nwo = g.label.toLowerCase(), i = nwo.indexOf('/'); return i > 0 ? [nwo.slice(0, i), nwo.slice(i + 1)] : [nwo, '']; };
  const cmp = (x, y) => x < y ? -1 : x > y ? 1 : 0;
  return [...groups.values()].sort((a, b) => { const [ao, an] = order(a), [bo, bn] = order(b); return cmp(ao, bo) || cmp(an, bn); });
}

/* A row's place among the others in its group: what waits on the person first, the longest waiting first. */
function workRowOrder(a, b) {
  const wait = r => r.s.waiting ? stampSecs(r.s.waiting.openedAt) || 0 : r.s.agentSession?.updatedAt || 0;
  return workStateOrder(a.st) - workStateOrder(b.st)
    || (WORKS_ON_PERSON(a) ? wait(a) - wait(b) : 0)
    || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
}
/* A row that is parked sorts with the ones that are only waiting to be looked at. */
const workStateOrder = st => STATE_ORDER[st] ?? STATE_ORDER.done;
const WORKS_ON_PERSON = r => r.st === 'waiting' || r.st === 'permission';

/* The repositories 「いまの仕事」 lists: one with something under way (a running session, a running hub) or waiting on the
   person (新着, a gate, a permission prompt, a parked task or a PR), one that could not be read, and the one holding the selection.
   An idle one is left out and comes back on its own; its board and the hub panel still list it. */
function workListedRepos(repos, rows, turns, chipsByRepo, keep = null) {
  const on = new Set();
  // A done or failed row the person has read no longer keeps its repository listed (#600); a gate, a permission wait, a park or a
  // PR still does, read or not.
  const open = r => (r.live || []).some(i => i.kind !== 'done' && i.kind !== 'failed');
  for (const r of [...rows, ...turns]) if (r.cls || WORKS_ON_PERSON(r) || WORK_RUNNING.includes(r.st) || open(r)) on.add(r.repo);
  for (const repo of repos) {
    if (repo.error || repo === keep || (chipsByRepo.get(repo) || []).some(c => c.s.present || c.waits > 0)) on.add(repo);
  }
  return on;
}

/* What 「いまの仕事」 draws of the document: the rows, turns, hub chips and repositories of the listed ones only. */
function workListedView(repos, rows, turns, chipsByRepo, keep = null) {
  const listed = workListedRepos(repos, rows, turns, chipsByRepo, keep);
  const mine = new Map([...chipsByRepo].filter(([repo]) => listed.has(repo)));
  return { rows: rows.filter(r => listed.has(r.repo)), turns: turns.filter(r => listed.has(r.repo)),
    chipsByRepo: mine, chips: [...mine.values()].flat(), repos: repos.filter(r => listed.has(r)) };
}

/* The list as a tree: owner → repository → parent issue → task, with the rows that belong to none under 「親なし」. A row is here
   wherever its state puts it, new or not, as is a task with nothing running (a turn row).
   The owner heading is shown even when there is only one owner. A hub is not a row here: it is a chip on the heading of its
   repository, or of the parent it runs. A node is `{ key, kind, label…, rows }` where `rows` are all the task rows below
   it, `items` what it holds in order (rows and nodes) and `hubs` the chips on its heading. */
function workTreeByParent(rows, bands = [], chipsByRepo = new Map(), repos = workRepos()) {
  const repoNodes = [];
  for (const repo of repos) {
    const mine = rows.filter(r => r.repo === repo && !r.isHub);
    const chips = chipsByRepo.get(repo) || [];
    if (!mine.length && !chips.length) continue;
    const items = [];
    const byParent = new Map();
    const loose = [];
    for (const r of mine) {
      const key = r.task?.parent;
      if (key) byParent.set(key, [...(byParent.get(key) || []), r]);
      else loose.push(r);
    }
    const parentNodes = [];
    for (const [key, list] of byParent) {
      const found = (repo.parents || []).find(p => p.key === key);
      const parent = found || { missing: true, key, url: key, children: [], total: list.length, merged: 0, stacked: false, hub: repo.carrier };
      parentNodes.push({ key: `parent:${repo.nwo}/${key}`, kind: 'parent', parent: { ...parent, repo }, rows: list.sort(workRowOrder), items: list.sort(workRowOrder).map(row => ({ row, place: 'tree' })), hubs: [] });
    }
    const rest = workPlaceHubChips(chips, parentNodes);
    items.push(...parentNodes);
    if (loose.length) {
      loose.sort(workRowOrder);
      items.push({ key: `none:${repo.nwo}`, kind: 'none', rows: loose, items: loose.map(row => ({ row, place: 'tree' })) });
    }
    repoNodes.push({ key: `repo:${repo.nwo}`, kind: 'repo', nwo: repo.nwo, rows: mine, items, hubs: rest, hub: chips.find(c => !c.hub.parent) || null });
  }
  return [...bands, ...workOwnerGroups(repoNodes)];
}

/* The list as boxes of one state each, in order, and inside each box by repository (`owner/name`, in name order, case
   ignored). A box with nothing in it is not drawn. The rows of 新着 have their action beside them, as in the band.
   A hub is a chip on its repository's heading, in 実行中 while it runs and in そのほか otherwise; its row is here only
   while it is new. */
function workTreeByState(rows, chips = []) {
  const tree = [];
  const listed = rows.filter(r => !r.isHub || r.cls === 'new');
  for (const box of WORK_BOXES) {
    const mine = listed.filter(r => workBoxOf(r) === box.id);
    const hubs = chips.filter(c => box.id === (c.s.present ? 'running' : 'other'));
    if (!mine.length && !hubs.length) continue;
    const place = box.id === 'new' ? 'new' : 'box';
    const items = workBoxRepoGroups(box.id, mine.sort(workRowOrder), place, hubs);
    tree.push({ key: `box:${box.id}`, kind: 'box', label: box.label, readAll: box.id === 'new', rows: mine, items });
  }
  return tree;
}

/* The children of a parent as the bar draws them: a segment each, then the ones the tracker counts and the board does not
   list as not started. At most so many, so a parent with a long tail is still a bar. */
function workSegments(p) {
  const segs = (p.children || []).map(c => ({ cls: WORK_PROGRESS[c.progress]?.[0] || 'not-started', title: `${c.id} ${WORK_PROGRESS[c.progress]?.[1] || ''}` }));
  const rest = Math.max(0, Math.min((p.total || 0) - segs.length, 60));
  for (let i = 0; i < rest; i++) segs.push({ cls: 'not-started', title: '未着手' });
  return segs;
}

/* The board a selection of `p` opens on: the hub that runs it when this server has its board, else the
   repository's board, which lists that hub's session too. */
const workParentBoard = p => p.repo ? workBoardOf(p.repo, p.hub) : p.hub;

/* The session the middle terminal shows for a parent: its hub's. */
function workParentSession(p) {
  const repo = p.repo;
  if (!repo) return null;
  return (repo.hubSessions || []).find(h => h.board === p.hub)?.session
    || (repo.rows || []).find(r => r.board === p.hub && r.session.kind === 'hub')?.session || null;
}

/* What a hub is doing, in words. A failed or unknown hub never reads as waiting: those are the states the person has to look at. */
const WORK_HUB_WORD = { working: '作業中', restarting: '作業中', done: '待機中', idle: '待機中', waiting: '待機中', permission: '待機中',
  failed: 'エラー', unknown: '状態不明', stopped: '動いていない', ended: '動いていない', none: '動いていない' };
const workHubWord = st => WORK_HUB_WORD[st] || '状態不明';

/* The hubs of a repository as the chips on its headings: its own first, then the parent-task hubs by key. A chip has the
   shape of a row, so pressing it selects the hub as pressing its row did, and says how many things wait on the person
   on it. A hub with no session in the document has no chip; an ended parent-task hub has none unless it waits. */
function workRepoHubChips(repo, now, judged) {
  const data = { now, repo: repo.nwo, hubs: repo.hubs || [], sessions: [] };
  const found = [...(repo.rows || []).filter(r => r.session.kind === 'hub').map(r => ({ r, parent: false })),
    ...(repo.hubSessions || []).map(r => ({ r, parent: true }))];
  const chips = [];
  for (const { r, parent } of found) {
    // `repo.hubs` says which hub it is; a session it does not list (the server falls back to the session's own key) still has a chip.
    const h = (repo.hubs || []).find(x => x.id === r.session.id) || { id: r.session.id, parent: parent || !!r.session.key, key: r.session.key ?? null, slug: r.board };
    let s = r.session;
    if (s.waiting && answeredGates.has(`${s.waiting.slug}/${s.waiting.id}`)) s = { ...s, waiting: null };
    // A hub that is not running reads as waiting while a gate is open on it (the server sets `waiting`): the chip says what it is.
    const state = sessionState(s, data);
    const st = !s.present && (state === 'waiting' || state === 'permission') ? restingState(s, data) : state;
    const ref = HUB_REF + s.id;
    const id = `${r.board}/${ref}`;
    const j = judged.get(id);
    const live = j?.live || [];
    if (h.parent && st === 'ended' && !live.length) continue;
    const board = workBoardOf(repo, r.board);
    // The wake goes to the hub's own board, so only a hub whose board this server has can be woken from here.
    const wakeBase = boards.some(b => b.slug === h.slug) ? `/b/${h.slug}` : null;
    chips.push({ id, key: `${board}/${ref}`, board, ref, repo, nwo: repo.nwo, hub: h, s, st, data, task: null, isHub: true, turn: false,
      live, cls: j?.cls || null, waits: live.length, word: workHubWord(st), wakeBase });
  }
  const own = c => c.hub.parent ? 1 : 0;
  const key = c => c.hub.key ?? '';
  return chips.sort((a, b) => own(a) - own(b) || (a.hub.parent && (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0)));
}

/* The chips given to the parent headings that a parent-task hub runs, one chip to a parent (the one of that parent's own key
   when there is one); a chip no heading takes is returned, to go on the repository's. */
function workPlaceHubChips(chips, parentNodes) {
  const pool = [...chips];
  const give = (node, chip) => { pool.splice(pool.indexOf(chip), 1); node.hubs = [chip]; };
  const real = parentNodes.filter(n => !n.parent.missing);
  // By the parent's own key first, so two parents run by one hub slug each get the chip that belongs to them; then a chip with no key by slug.
  for (const node of real) {
    const chip = pool.find(c => c.hub.parent && c.hub.slug === node.parent.hub && c.hub.key === node.parent.key);
    if (chip) give(node, chip);
  }
  for (const node of real) {
    if (node.hubs.length) continue;
    // Only a chip with no key of its own: a keyed one that no parent matched goes on the repository's heading.
    const chip = pool.find(c => c.hub.parent && !c.hub.key && c.hub.slug === node.parent.hub);
    if (chip) give(node, chip);
  }
  return pool;
}

/* How many things the hubs on the headings below this node wait on the person with: what a folded heading says of them,
   as it hides their chips. */
function workHiddenHubWaits(node) {
  let n = 0;
  for (const it of node.items || []) {
    if (it.row) continue;
    n += (it.hubs || []).reduce((sum, c) => sum + c.waits, 0) + workHiddenHubWaits(it);
  }
  return n;
}

/* What the address selects: the row (or parent) and the session whose terminal is in the middle. */
function workSelected() {
  if (!work.doc || !nav.task || !nav.board || nav.board === 'all') return null;
  const ref = nav.task;
  if (isParentRef(ref)) {
    const parent = parentOfRef(ref);
    return parent ? { parent, session: workParentSession(parent) } : null;
  }
  for (const repo of workRepos()) {
    let found = null;
    for (const r of repo.rows || []) {
      if (workBoardOf(repo, r.board) !== nav.board) continue;
      const mine = isHubRef(ref) ? r.session.kind === 'hub' && HUB_REF + r.session.id === ref
        : isSessRef(ref) ? r.session.id === sessIdOfRef(ref) : r.task?.id === ref;
      // Of several sessions of one task, the one the list shows.
      if (mine && (!found || workBetter(r.session, found.session))) found = r;
    }
    if (found) return { row: found, repo, session: found.session };
    // A gate that only has a row of its own: nothing to open beside it.
    if (isGateRef(ref)) {
      const slug = ref.slice(WORK_GATE_REF.length).split('/')[0];
      if (workBoardOf(repo, slug) === nav.board) return { row: { board: slug, session: null, task: null }, repo, session: null };
    }
    // A task that waits on the person with nothing running: there is no session to show.
    if (!isHubRef(ref) && !isSessRef(ref)) {
      const t = (repo.turns || []).find(x => x.task?.id === ref && workBoardOf(repo, x.board) === nav.board);
      if (t) return { row: { board: t.board, session: null, task: t.task }, repo, session: null };
    }
    if (isHubRef(ref)) {
      const h = (repo.hubSessions || []).find(x => workBoardOf(repo, x.board) === nav.board && HUB_REF + x.session.id === ref);
      if (h) return { row: { board: h.board, session: h.session, task: null }, repo, session: h.session };
    }
  }
  return null;
}

const isGateRef = ref => typeof ref === 'string' && ref.startsWith(WORK_GATE_REF);

/* Whether the address names this row: its board and ref. */
const workIsSelected = r => !!nav.task && !!nav.board && nav.board !== 'all' && !isParentRef(nav.task) && r.board === nav.board && r.ref === nav.task;

/* ── the document ── */

let endState = null;
/* The gate to show on the open task's tab, once its panel is drawn: `{ board, task, gate }` (selectWorkRow). Replaced by the
   next selectWorkRow, and dropped by the next panel draw whose address is off that board's task. */
let wantedGate = null;
/* A session that finishes or fails, as the document shows it: one desktop notification each, for the kinds the person
   chose (「通知」), and not for what was already there when the page opened. The keys are learned whether or not the
   browser lets the page ring. Waits are rung by checkNewGates/checkNewWaits (core.js). */
function checkNewEnds(fetchedAt) {
  // The gates and waits that were rung for before the document knew their task (core.js).
  flushNotifyPending(fetchedAt);
  const result = notifyEndEvents(workEntries(work.doc, key => answeredGates.has(key)), endState,
    { prefs: prefs.notify, openId: work.open?.id, visible: document.visibilityState === 'visible',
      okRepos: (work.doc.repos || []).filter(r => !r.error).map(r => r.nwo) });
  endState = result;
  const { events } = result;
  if (!(window.Notification && Notification.permission === 'granted')) return;
  for (const { kind, entry, key } of events) {
    const { title, body } = notifyContent(kind, entry);
    const n = new Notification(title, { body, tag: `adj-${kind}-${key}` });
    n.onclick = () => openNotified(entry, n);
  }
}

async function refreshWork(force = false) {
  if (!multiBoard) return;
  if (work.busy) {
    // An answer to something just done must not be lost to the round already out.
    if (force) work.again = true;
    return;
  }
  work.busy = true;
  try {
    const fetchedAt = Date.now();
    const doc = await boardApi('', '/api/work');
    const failed = work.error != null;
    work.error = null;
    work.doc = doc;
    checkNewEnds(fetchedAt);
    workTrackParks();
    // The sidebar counts what is new on every view; the list is drawn on its own.
    renderWorkBadge();
    if (pendingGate && view === 'work') openPendingGate();
    if (view !== 'work') return;
    const json = JSON.stringify(workSig(doc));
    const minute = Math.floor((doc.now || 0) / 60);
    if (!force && !failed && json === work.json && minute === work.minute) return;
    work.json = json;
    work.minute = minute;
    renderWorkView();
    // The panel reads the document for a parent, and the address may have named one that was not here yet.
    if (pendingTask && isParentRef(pendingTask) && !parentOfRef(pendingTask)) {
      // A parent the document does not have is one the panel says is not here.
      selectedTaskId = pendingTask;
      pendingTask = null;
    } else applyPendingTask();
    renderTaskPanel();
  } catch (e) {
    work.error = e.message;
    // The gates and waits that were to be rung with the document must not wait for one that does not come.
    flushNotifyPending();
    if (view === 'work') renderWorkView();
  } finally {
    work.busy = false;
    if (work.again) {
      work.again = false;
      refreshWork(true);
    }
  }
}

/* ── the list ── */

const workGlyphHtml = (st, extra = '') => {
  const [icon, label] = WORK_GLYPH[st] || WORK_GLYPH.idle;
  return `<span class="material-symbols-outlined wk-glyph ${st}${extra}" role="img" aria-label="${esc(label)}" title="${esc(label)}">${icon}</span>`;
};

const workPercent = n => Math.max(0, Math.min(100, Math.round(Number(n) || 0)));

/* One row, in the order the issue gives: the state, the agent and its model and context, the title, the PR and branch
   and how long ago the state changed, the sub-agents, the diff, what it asks, what it is running, and the gate. */
function workRowHtml(r, cell = '', band = false) {
  const { s, st, data, task } = r;
  const a = s.present ? s.agentSession || {} : {};
  const ctx = a.contextPercent != null ? workPercent(a.contextPercent) : null;
  const agent = [esc(s.agent), a.model ? esc(a.model) : ''].filter(Boolean).join(' · ');
  const bar = ctx == null ? '' : `<span class="wk-ctx${ctx >= 90 ? ' crit' : ctx >= 70 ? ' warn' : ''}" title="コンテキスト ${ctx}%"><span class="wk-ctx-bar"><i style="width:${ctx}%"></i></span>${ctx}%</span>`;
  const label = sessionLabel(s, false, data);
  const title = task?.title || label.text;
  const prNumber = task ? prRefNumber(task.pr) : s.branchPr?.number;
  const since = a.updatedAt ?? s.phaseAt;
  const age = since != null && data.now != null ? agoLabel(minutesSince(since, data.now)) : '';
  const subs = s.present ? (a.subagents || []) : [];
  const u = s.uncommitted;
  const diff = u && (u.insertions || u.deletions) ? `<span class="add">+${Number(u.insertions) || 0}</span> <span class="del">-${Number(u.deletions) || 0}</span>` : '';
  const meta = [
    workAwayChip(r.away),
    prNumber ? `#${esc(prNumber)}` : '',
    s.branch ? esc(s.branch) : '',
    age ? esc(age) : '',
    subs.length ? `<span class="sub">サブエージェント ${subs.length}</span>` : '',
    diff,
  ].filter(Boolean).map(x => `<span>${x}</span>`).join('');
  const asks = !s.waiting && agentStateOf(s) === 'permission' && a.request ? `<span class="wk-line ask">${esc(requestText(s))}</span>` : '';
  // In 新着 a row says what the agent last said when it asks for nothing. A parked row says so wherever it is listed.
  const said = band && !asks && !s.waiting && a.lastMessage ? workSaidHtml(a.lastMessage) : '';
  const parked = !s.waiting ? workParkedHtml(r) : '';
  const doing = st === 'working' && a.activity ? `<span class="wk-line">${esc(a.activity)}</span>` : '';
  const tree = st === 'working' && subs.length
    ? subs.slice(0, WORK_SUBAGENTS_SHOWN).map(x => `<span class="wk-line tree">${esc(`└ ${x.type || 'サブエージェント'}${x.activity ? ` ${x.activity}` : ''}`)}</span>`).join('')
      + (subs.length > WORK_SUBAGENTS_SHOWN ? `<span class="wk-line tree">${esc(`└ ほか ${subs.length - WORK_SUBAGENTS_SHOWN} 件`)}</span>` : '') : '';
  // A worker stopped at a gate looks done to its agent: the line is what says the ball is the person's.
  const gate = s.waiting ? `<span class="wk-gate"><span class="material-symbols-outlined" aria-hidden="true">pending_actions</span>${esc(kindOf(s.waiting.kind)[0])} · ${esc(workGateWho(task))}</span>` : '';
  return `<button type="button" class="wk-row ${st}" data-wk="${esc(r.key)}"${cell} title="${esc(sessionTip(s, st, data))}">
    ${workGlyphHtml(st)}<span class="wk-main">
      ${agent || bar ? `<span class="wk-agent">${agent}${bar}</span>` : ''}
      <span class="wk-title">${r.isHub ? '<span class="wk-tag">hub</span>' : ''}${esc(title)}</span>
      ${meta ? `<span class="wk-meta">${meta}</span>` : ''}${asks}${said}${doing}${tree}${gate}${parked}
    </span></button>`;
}

/* How many things happened since the person left the task (#556): nothing is drawn for none. */
const workAwayChip = n => n > 0 ? `<span class="wk-away" aria-label="離れていた間に ${n} 件" title="離れていた間に ${n} 件">+${n}</span>` : '';

/* The first line of what the agent said at the end of its turn, cut to one line by the stylesheet. */
const workSaidHtml = message => {
  const line = String(message).split('\n').map(x => x.trim()).find(Boolean);
  return line ? `<span class="wk-line msg">${esc(line)}</span>` : '';
};

/* The line of a parked row, with since when; nothing for a row that is not parked. */
function workParkedHtml(r) {
  const item = (r.live || []).find(i => i.kind === 'parked');
  return item ? `<span class="wk-line parked">${esc(workParkedText(item, r.data?.now ?? null))}</span>` : '';
}

/* The words after a gate's kind on a row: whose turn it is, or that it is parked (the gate stays open and answerable). */
const workGateWho = task => parkOf(task) ? `置いている — ${parkLabel(parkOf(task))}` : 'あなたの判定待ち';

/* A row for something that waits on the person and has no session row of its own: a task whose PR is theirs, or whose gate
   opened with nothing running, or a session the list does not show. */
function workTurnRowHtml(r, cell = '', band = false) {
  const e = r.entry;
  const title = r.task?.title || (e.session ? sessionLabel(e.session, false, r.data).text : e.gateTitle || r.ref);
  const prNumber = r.task ? prRefNumber(r.task.pr) : null;
  const newest = workNewest(r);
  const age = newest && r.data.now != null ? agoLabel(minutesSince(newest, r.data.now)) : '';
  const meta = [workAwayChip(r.away), prNumber ? `#${esc(prNumber)}` : '', age ? esc(age) : ''].filter(Boolean).map(x => `<span>${x}</span>`).join('');
  const gate = r.live.find(i => i.kind === 'gate');
  const pr = r.live.find(i => i.kind === 'pr');
  const lines = (gate ? `<span class="wk-gate"><span class="material-symbols-outlined" aria-hidden="true">pending_actions</span>${esc(kindOf(gate.gate)[0])} · ${esc(workGateWho(r.task))}</span>` : '')
    + (pr ? `<span class="wk-line ask">${esc(WORK_PR_NOW[pr.turn] || 'PR があなたの番です')}</span>` : '');
  const st = r.st === 'parked' ? 'parked' : 'waiting';
  return `<button type="button" class="wk-row ${st}" data-wk="${esc(r.key)}"${cell}>
    ${workGlyphHtml(st)}<span class="wk-main">
      <span class="wk-title">${r.isHub ? '<span class="wk-tag">hub</span>' : ''}${esc(title)}</span>
      ${meta ? `<span class="wk-meta">${meta}</span>` : ''}${lines}${gate ? '' : workParkedHtml(r)}
    </span></button>`;
}

/* A row as the list draws it in `place`: a row of 新着 (the band, and that box of 「状態」) has its 既読 button beside it, and the
   row itself stays one button, so selecting is the same everywhere. */
function workItemHtml(r, place) {
  const band = place === 'new';
  const cellKey = `${place}/${r.key}`;
  const cell = ` data-wk-cell="${esc(cellKey)}"`;
  if (!band) return r.turn ? workTurnRowHtml(r, cell) : workRowHtml(r, cell);
  const act = (attr, icon, label) => `<button type="button" class="wk-act" ${attr}="${esc(r.id)}" title="${esc(label)}" aria-label="${esc(label)}"><span class="material-symbols-outlined" aria-hidden="true">${icon}</span></button>`;
  const acts = act('data-wk-read', 'done', '既読にする');
  return `<div class="wk-band-row"${cell}>${r.turn ? workTurnRowHtml(r, '', true) : workRowHtml(r, '', true)}<span class="wk-acts">${acts}</span></div>`;
}

/* How many rows are in each state, for a header that is folded. */
function workSummaryHtml(rows, hubWaits = 0) {
  const counts = new Map();
  for (const r of rows) counts.set(r.st, (counts.get(r.st) || 0) + 1);
  const hubLabel = esc(`hub があなたを待っています ${hubWaits} 件`);
  const hub = hubWaits > 0 ? `<span role="img" aria-label="${hubLabel}" title="${hubLabel}"><span class="material-symbols-outlined" aria-hidden="true">account_tree</span><span class="material-symbols-outlined wk-glyph waiting" aria-hidden="true">front_hand</span>${hubWaits}</span>` : '';
  return `<span class="wk-sum">${[...counts].sort((a, b) => workStateOrder(a[0]) - workStateOrder(b[0]))
    .map(([st, n]) => `<span>${workGlyphHtml(st)}${n}</span>`).join('')}${hub}</span>`;
}

/* The wake button beside a hub's chip while it has unseen messages, with the reason a press typed nothing. It is the chip's sibling,
   never inside it, and follows the rules of the board's own (`hubWakeBlocked`). */
function workHubWakeHtml(c) {
  const unseen = c.hub.unseen || 0;
  const why = hubWakeWhy(c.hub);
  if (!unseen) return '';
  const off = hubWakeBlocked(c.hub, c.wakeBase);
  const label = `hub を起こす（受信箱の未確認 ${unseen} 件）`;
  return `<button type="button" class="wk-hub-wake" data-wk-wake="${esc(c.key)}" data-wake-slug="${esc(c.hub.slug)}"${off ? ' disabled' : ''} aria-label="${esc(label)}" title="${esc(off || label)}"><span class="material-symbols-outlined" aria-hidden="true">notifications_active</span>${unseen}</button><span class="hub-wake-why wk-hub-why" role="status">${esc(why)}</span>`;
}

/* The hubs on a heading, each a button of its own beside the heading's main button (never inside it). It says the hub, what it
   is doing, and, when it waits on the person, how many things; pressing it opens the hub (a stopped one offers 「hub を起動」
   there). A hub of a parent task is named by its key where the heading is not that parent's. */
function workHubChipsHtml(chips, where, named) {
  if (!chips?.length) return '';
  return `<span class="wk-hubs">${chips.map(c => {
    const off = ['stopped', 'ended', 'none'].includes(c.st);
    // Only a hub that is not there is started from its panel; an ended parent-task hub is closed there.
    const startable = c.st === 'stopped' || c.st === 'none';
    const name = named && c.hub.parent && c.hub.key ? `hub ${c.hub.key}` : 'hub';
    const tip = `${where ? `${where} の ` : ''}${name}: ${c.word}${c.waits ? `。あなたを待っているもの ${c.waits} 件` : ''}`;
    const wait = c.waits ? `<span class="wk-hub-wait${c.cls === 'new' ? ' new' : ''}" role="img" aria-label="${esc(`あなたを待っているもの ${c.waits} 件`)}" title="${esc(`あなたを待っているもの ${c.waits} 件`)}"><span class="material-symbols-outlined" aria-hidden="true">front_hand</span>${c.waits}</span>` : '';
    const chip = `<button type="button" class="wk-hub${c.st === 'working' || c.st === 'restarting' ? ' working' : off ? ' off' : ''}" data-wk-hub="${esc(c.key)}" aria-label="${esc(`${tip}。開く`)}" title="${esc(`${tip}${startable ? '。開くと hub を起動できます' : ''}`)}"><span class="material-symbols-outlined" aria-hidden="true">account_tree</span><span>${esc(name)}</span><span class="wk-hub-word">${esc(c.word)}</span>${wait}</button>`;
    return chip + workHubWakeHtml(c);
  }).join('')}</span>`;
}

function workFoldButton(key, folded, name) {
  const verb = folded ? '開く' : 'たたむ';
  return `<button type="button" class="wk-fold" data-wk-fold="${esc(key)}" aria-expanded="${!folded}" aria-label="${esc(`${name}を${verb}`)}" title="${esc(`${name}を${verb}`)}"><span class="material-symbols-outlined" aria-hidden="true">expand_more</span></button>`;
}

/* A repository heading that holds a hub chip and nothing else: there is nothing to fold. */
const workChipsOnly = node => !node.items.length && !node.empty && !!node.hubs?.length;

/* A node's header. A folded one says how many rows are in each state. */
function workHeadHtml(node, folded) {
  const sum = folded ? workSummaryHtml(node.rows, workHiddenHubWaits(node)) : '';
  const fold = (key, name) => workChipsOnly(node) ? '<span class="wk-fold-gap"></span>' : workFoldButton(key, folded, name);
  // A count of task rows: a heading with none says nothing (a hub alone is a chip), except the band, which says it is empty.
  const count = node.rows.length || node.kind === 'band' ? `<span class="wk-count">${node.rows.length}</span>` : '';
  if (node.kind === 'parent') {
    const p = node.parent;
    const segs = workSegments(p);
    const merged = p.merged || 0;
    const total = Math.max(p.total || 0, (p.children || []).length);
    const name = `${p.number ? `#${p.number}` : p.key}${p.title ? ` ${p.title}` : ''}`;
    const current = nav.task === PARENT_REF + p.key;
    // A parent the document does not list has nothing to open: its header is plain text.
    const [open, close, attrs] = p.missing ? ['span', 'span', ''] : ['button', 'button', ` type="button" data-wk-parent="${esc(p.key)}"${current ? ' aria-current="true"' : ''}`];
    return `${fold(node.key, name)}<${open} class="wk-head-main"${attrs} title="${esc(`${name}\nこの親 Issue を動かしている hub のターミナルを開く`)}">
      <span class="wk-head-name">${p.number ? `<span class="key">#${p.number}</span>` : `<span class="key">${esc(p.key)}</span>`}${esc(p.title || '')}</span>
      <span class="wk-head-sub"><span class="wk-bar" role="img" aria-label="${esc(`${merged} / ${total} マージ`)}">${segs.map(x => `<i class="${x.cls}" title="${esc(x.title)}"></i>`).join('')}</span>
      <span>${merged} / ${total} マージ</span>${p.stacked ? '<span class="wk-stack">stack</span>' : ''}${sum}</span></${close}>${workHubChipsHtml(node.hubs, name)}${count}`;
  }
  const name = node.kind === 'repo' ? node.nwo : node.kind === 'none' ? '親なし' : node.label;
  // 「処理したら次へ」: after a gate is answered or closed, the next row of 新着 opens (the same setting for every header that has it).
  const readAll = node.readAll && node.rows.length
    ? `<label class="wk-next" title="判定を返したら、新着の次の行を開く"><input type="checkbox" data-wk-advance><span>処理したら次へ</span></label><button type="button" class="wk-act" data-wk-read-all title="新着をすべて既読にする" aria-label="新着をすべて既読にする"><span class="material-symbols-outlined" aria-hidden="true">done_all</span></button>` : '';
  return `${fold(node.key, name)}<span class="wk-head-main"><span class="wk-head-name">${esc(name)}</span>${sum ? `<span class="wk-head-sub">${sum}</span>` : ''}</span>${workHubChipsHtml(node.hubs, name, node.kind === 'repo' || node.kind === 'brepo')}${count}${readAll}`;
}

/* The tree as markup, and the pieces of it that are redrawn alone: `cells` maps a row's or header's key to its own html. */
function workTreeHtml(items, cells, rowsByKey, folded) {
  return items.map(it => {
    if (it.row) {
      // A row can be in the list twice (a band and the tree): the cell is the row in its place.
      const cell = `row:${it.place}/${it.row.key}`;
      cells.set(cell, workItemHtml(it.row, it.place));
      rowsByKey.set(it.row.key, it.row);
      return cells.get(cell);
    }
    const isFolded = folded.has(it.key);
    cells.set(`head:${it.key}`, workHeadHtml(it, isFolded));
    return `<section class="wk-group" data-wk-g="${esc(it.key)}"><div class="wk-head ${it.kind}" data-wk-head="${esc(it.key)}">${cells.get(`head:${it.key}`)}</div>`
      + (isFolded || workChipsOnly(it) ? '' : `<div class="wk-body">${it.items.length ? workTreeHtml(it.items, cells, rowsByKey, folded) : it.empty ? `<div class="wk-empty">${esc(it.empty)}</div>` : ''}</div>`) + '</section>';
  }).join('');
}

/* The nodes with their keys and what they hold, which is what decides a rebuild: a row or a header that only changed
   its words is replaced alone. */
function workShape(items, folded) {
  return items.map(it => it.row ? `${it.place}:${it.row.key}` : [it.key, folded.has(it.key), it.kind === 'parent' ? 1 : 0, (it.hubs || []).map(c => c.key), folded.has(it.key) ? [] : workShape(it.items, folded)]);
}

/* How to find again what has the keyboard focus in the list: a row, a fold button or a parent's header. */
const workHeldSel = el => el?.dataset?.wk != null ? `[data-wk="${CSS.escape(el.dataset.wk)}"]`
  : el?.dataset?.wkRead != null ? `[data-wk-read="${CSS.escape(el.dataset.wkRead)}"]`
  : el?.dataset?.wkReadAll != null ? '[data-wk-read-all]'
  : el?.dataset?.wkAdvance != null ? '[data-wk-advance]'
  : el?.dataset?.wkFold != null ? `[data-wk-fold="${CSS.escape(el.dataset.wkFold)}"]`
  : el?.dataset?.wkWake != null ? `[data-wk-wake="${CSS.escape(el.dataset.wkWake)}"]`
  : el?.dataset?.wkHub != null ? `[data-wk-hub="${CSS.escape(el.dataset.wkHub)}"]`
    : el?.dataset?.wkParent != null ? `[data-wk-parent="${CSS.escape(el.dataset.wkParent)}"]` : null;

/* A row replaced by the one its html makes, keeping its selection and the keyboard focus. */
function workSwap(old, html) {
  const held = old.contains(document.activeElement) ? workHeldSel(document.activeElement) : null;
  const node = htmlNode(html);
  if (old.hasAttribute('aria-current')) node.setAttribute('aria-current', 'true');
  old.replaceWith(node);
  // The marks follow in `markWorkSelection`; the focus is found again in the new markup.
  if (held) (node.matches(held) ? node : node.querySelector(held))?.focus({ preventScroll: true });
  return node;
}

/* A header's own markup replaced, keeping the keyboard focus of what was in it. */
function workSwapHead(head, html) {
  const held = head.contains(document.activeElement) ? workHeldSel(document.activeElement) : null;
  head.innerHTML = html;
  if (held) head.querySelector(held)?.focus({ preventScroll: true });
}

/* The rows of 新着 in the order the tree draws them: a band's, or a box's of 「状態」. */
function workNewOrder(items) {
  const out = [];
  for (const it of items) {
    if (it.row) { if (it.place === 'new') out.push(it.row.id); } else out.push(...workNewOrder(it.items));
  }
  return out;
}

function renderWorkList() {
  drawWorkList();
  // Whatever redrew the list (a fold, the other grouping) must not lose the mark.
  markWorkSelection();
  // The box is set apart from the markup, so a toggle does not redraw the header.
  for (const box of wk('wk-groups')?.querySelectorAll('[data-wk-advance]') || []) box.checked = !!prefs.reviewNext;
}

function drawWorkList() {
  const root = wk('wk-groups');
  if (!root) return;
  if (!work.doc) {
    const text = work.error ? `読み込めませんでした: ${work.error}` : '読み込み中…';
    if (work.structure !== `text:${text}`) {
      work.structure = `text:${text}`;
      work.cells = new Map();
      root.innerHTML = `<div class="wk-empty">${esc(text)}</div>`;
    }
    return;
  }
  const { rows: allRows, turns: allTurns, judged } = workListRows();
  const allChips = new Map(workRepos().map(r => [r, workRepoHubChips(r, work.doc.now, judged)]));
  const sel = workSelected();
  const { rows, turns, chipsByRepo, chips, repos } = workListedView(workRepos(), allRows, allTurns, allChips, sel?.repo || sel?.parent?.repo || null);
  const byState = prefs.workGroup === 'state';
  // The rows with no session row of their own (a task with nothing running) are in the band, the boxes and the tree alike.
  const tree = byState ? workTreeByState([...rows, ...turns], chips) : workTreeByParent([...rows, ...turns], workBands([...rows, ...turns]), chipsByRepo, repos);
  work.newOrder = workNewOrder(tree);
  const folded = new Set(prefs.workFolded);
  for (const b of wk('work-view').querySelectorAll('[data-wk-group]')) b.setAttribute('aria-pressed', String(b.dataset.wkGroup === prefs.workGroup));
  const cells = new Map();
  const rowsByKey = new Map();
  // A poll that failed after one that worked, and a repository none of whose boards could be read: said above
  // the list, which is the last that was read.
  const notes = [
    ...(work.error ? [`最新の状態を読めませんでした（表示は前回のものです）: ${work.error}`] : []),
    ...workRepos().filter(r => r.error).map(r => `${r.nwo}: ${r.error}`),
  ];
  const html = notes.map(n => `<div class="wk-notice" role="alert">${esc(n)}</div>`).join('') + workTreeHtml(tree, cells, rowsByKey, folded);
  work.rowsByKey = rowsByKey;
  work.hubsByKey = new Map(chips.map(c => [c.key, c]));
  const structure = JSON.stringify([prefs.workGroup, workShape(tree, folded), notes]);
  if (structure === work.structure) {
    // Only what changed in its own words is drawn again, so a poll does not take the scroll or the focus.
    for (const [key, cell] of cells) {
      if (work.cells.get(key) === cell) continue;
      const [kind, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
      const old = kind === 'row' ? [...root.querySelectorAll('[data-wk-cell]')].find(el => el.dataset.wkCell === id)
        : [...root.querySelectorAll('.wk-head[data-wk-head]')].find(el => el.dataset.wkHead === id);
      if (!old) continue;
      if (kind === 'row') workSwap(old, cell);
      else workSwapHead(old, cell);
    }
    work.cells = cells;
    return;
  }
  work.structure = structure;
  work.cells = cells;
  const scroll = root.closest('.wk-list-scroll');
  const top = scroll.scrollTop;
  const at = document.activeElement;
  const held = root.contains(at) ? workHeldSel(at) : null;
  root.innerHTML = rows.length || turns.length || notes.length || chips.length ? html : '<div class="wk-empty">いま動いている仕事はありません</div>';
  scroll.scrollTop = top;
  if (held) root.querySelector(held)?.focus({ preventScroll: true });
}

/* What the person's selection is marked on. */
function markWorkSelection() {
  const parent = nav.task && isParentRef(nav.task) ? parentKeyOfRef(nav.task) : null;
  const root = wk('wk-groups');
  if (!root) return;
  for (const el of root.querySelectorAll('.wk-row[data-wk]')) {
    const r = work.rowsByKey.get(el.dataset.wk);
    if (r && workIsSelected(r)) el.setAttribute('aria-current', 'true'); else el.removeAttribute('aria-current');
  }
  for (const el of root.querySelectorAll('[data-wk-hub]')) {
    const c = work.hubsByKey.get(el.dataset.wkHub);
    if (c && workIsSelected(c)) el.setAttribute('aria-current', 'true'); else el.removeAttribute('aria-current');
  }
  for (const el of root.querySelectorAll('[data-wk-parent]')) {
    if (el.dataset.wkParent === parent) el.setAttribute('aria-current', 'true'); else el.removeAttribute('aria-current');
  }
}

/* ── the rate limits ── */

function workRateHtml(rl, now) {
  if (!rl) return '';
  const accounts = rl.accounts || [];
  const win = (label, w) => {
    if (!w || w.usedPercent == null) return '';
    const used = workPercent(w.usedPercent);
    const left = w.resetsAt != null && now != null ? w.resetsAt - now : null;
    const reset = left == null ? '' : left <= 0 ? ' · 更新待ち' : ` · あと${minutesLabel(Math.ceil(left / 60))}でリセット`;
    return `<span class="wk-rate-win" title="${esc(`${label}: ${used}% 使用${reset}`)}">${esc(label)}<span class="wk-ctx${used >= 90 ? ' crit' : used >= 70 ? ' warn' : ''}"><span class="wk-ctx-bar"><i style="width:${used}%"></i></span>${used}%</span>${esc(reset)}</span>`;
  };
  const lines = accounts.map(a => {
    const name = accounts.length > 1 ? `${a.agent} · ${baseName(a.configDir) || a.configDir}` : a.agent;
    const seen = a.lastEventAt != null && now != null ? `最後に聞いたのは${agoLabel(minutesSince(a.lastEventAt, now))}` : '';
    return `<div class="wk-rate-acct" title="${esc(seen)}"><b>${esc(name)}</b>${win('5時間', a.fiveHour)}${win('7日', a.sevenDay)}</div>`;
  }).filter(x => x.includes('wk-rate-win'));
  const error = rl.error ? `<div class="wk-rate-err">${esc(`レート制限を読めません: ${rl.error}`)}</div>` : '';
  return lines.join('') + error;
}

function renderWorkRate() {
  const el = wk('wk-rate');
  if (!el) return;
  const html = work.doc ? workRateHtml(work.doc.rateLimits, work.doc.now) : '';
  if (el.dataset.sig === html) return;
  el.dataset.sig = html;
  el.innerHTML = html;
}

/* ── the middle terminal ── */

const workTermPart = {};
function setWorkTermPart(name, el, html) {
  if (!el || workTermPart[name] === html) return;
  workTermPart[name] = html;
  el.innerHTML = html;
}

function workTermHeadHtml(sel, free = null) {
  if (!sel && free) {
    const label = sessionLabel(free);
    return `<div class="wk-term-title">${esc(label.text)}</div><div class="wk-term-sub">${esc([free.branch, free.worktree && baseName(free.worktree)].filter(Boolean).join(' · '))}</div>`;
  }
  if (!sel) return '';
  if (sel.parent) {
    const p = sel.parent;
    return `<div class="wk-term-title">${esc(`${p.number ? `#${p.number} ` : ''}${p.title || p.key}`)}</div><div class="wk-term-sub">${esc(p.key)} · この親 Issue を動かしている hub</div>`;
  }
  const r = sel.row;
  const data = { now: work.doc?.now, repo: sel.repo.nwo, hubs: sel.repo.hubs || [], sessions: [] };
  const title = r.task?.title || (r.session ? sessionLabel(r.session, false, data).text : r.task?.id || '');
  const sub = [r.session?.branch, r.task?.id, sel.repo.nwo].filter(Boolean).join(' · ');
  return `<div class="wk-term-title">${esc(title)}</div><div class="wk-term-sub">${esc(sub)}</div>`;
}

/* The terminal of what is selected, in the middle. A session of the board that was selected is the page's
   own (its actions and its socket are that board's); one that board does not list is shown without them. */
function renderWorkTerm() {
  if (view !== 'work') return;
  const sel = workSelected();
  // A worktree with no process (the hub's 「動いていない worktree」) is no row of the list, but the board lists it.
  const free = !sel && isSessRef(nav.task) ? sessOfRef(nav.task) : null;
  const ds = sel?.session || null;
  const own = ds ? (state.sessions || []).find(x => x.id === ds.id) || null : free;
  const s = own || ds;
  setWorkTermPart('head', wk('wk-term-head'), workTermHeadHtml(sel, free));
  // Session ids are the board's own (every repository has a `hub`): a socket is not carried to another board.
  if (workTerm.board !== nav.board) disposeTermSlot(workTerm);
  workTerm.board = nav.board;
  syncTermSlot(workTerm, `${nav.board}/${s?.id || ''}`, own, own ? 'term' : 'none');
  setWorkTermPart('bar', wk('wk-term-bar'), s && own && hasSession(own) ? termBarHtml(own, workTerm, true) : '');
  const ph = wk('wk-term-ph');
  ph.hidden = !!workTerm.term;
  const loaded = state.now != null;
  const text = workTerm.term ? ''
    : !nav.task || nav.board === 'all' ? '<div class="tp-muted">左の一覧から選ぶと、ここにそのセッションのターミナルが開きます</div>'
      : !work.doc ? '<div class="tp-muted">読み込み中…</div>'
      : free ? termPlaceholderHtml(free)
      : !sel ? '<div class="tp-muted">選んだ仕事は、いまの一覧にありません</div>'
        : !ds ? `<div class="tp-muted">${sel.parent ? 'この親 Issue を動かしている hub のセッションが見つかりません' : 'この仕事にはセッションがありません'}</div>`
          : !loaded ? '<div class="tp-muted">接続しています…</div>'
            : !own ? '<div class="tp-muted">このボードにはこのセッションが見えないため、ターミナルを開けません</div>'
              : termPlaceholderHtml(own);
  setWorkTermPart('ph', ph, text);
}

/* The bar over the middle terminal: the session's own actions, as the panel's terminal bar has them. */
wk('wk-term-bar').addEventListener('click', e => {
  const b = e.target.closest('[data-sess-act], [data-tp-reconnect]');
  if (!b || b.disabled) return;
  const id = workSelected()?.session?.id || (isSessRef(nav.task) ? sessIdOfRef(nav.task) : null);
  const s = id ? (state.sessions || []).find(x => x.id === id) : null;
  if (!s) return;
  if (b.dataset.tpReconnect != null) {
    workTerm.reconnect = true;
    return renderWorkTerm();
  }
  // A session resumed or started from here is connected to once its window exists (syncTermSlot).
  if (b.dataset.sessAct === 'resume' || b.dataset.sessAct === 'hub-start') workTerm.reconnect = true;
  runSessionAction(b.dataset.sessAct, s);
});

/* The one place the middle terminal is asked to take the keyboard: 「ターミナルで話す」. */
function focusWorkTerm() {
  if (workTerm.term) workTerm.term.focus();
  else note('ターミナルは開いていません', true, '左の一覧から、セッションのある仕事を選んでください');
}

/* The marks and the terminal of what the address names. */
function syncWorkSelection() {
  workTrack();
  markWorkSelection();
  renderWorkTerm();
}

function renderWorkView() {
  if (view !== 'work') return;
  renderWorkList();
  renderWorkRate();
  syncWorkSelection();
}

/* ── choosing ── */

/* An old link to a gate (`/review?item=<board>/<id>`, `#gate/<id>`), once the document says where the gate is: the row it is
   on, else the panel of the gate itself. */
function openPendingGate() {
  const ref = pendingGate;
  if (!ref || !work.doc) return;
  pendingGate = null;
  if (openGateRow(ref, { replace: true })) return;
  const slug = ref.slice(0, ref.indexOf('/'));
  go({ board: slug, view: 'work', task: WORK_GATE_REF + ref, pane: 'detail' }, { replace: true });
}

/* Select the row of the list whose entry holds the gate `key` (`<board>/<id>`), with that gate shown: false when no row does. */
function openGateRow(key, { replace = false } = {}) {
  if (!work.doc) return false;
  const { rows, turns, judged } = workListRows();
  const entry = [...judged.values()].map(j => j.entry).find(e => e.items.some(i => i.kind === 'gate' && i.key === key));
  const row = entry && [...rows, ...turns].find(r => r.id === entry.id);
  if (!row) return false;
  const item = entry.items.find(i => i.kind === 'gate' && i.key === key);
  selectWorkRow(row, { replace, gate: { id: key.slice(key.indexOf('/') + 1), kind: item.gate } });
  return true;
}

function selectWorkRow(r, { replace = false, gate: wanted = null } = {}) {
  // The gate a notification or a link is about is the one shown in its tab (renderTaskPanel picks it).
  wantedGate = wanted && r.task ? { board: r.board, task: r.task.id, gate: wanted.id } : null;
  // A task opens on the tab its open gate is judged in, else on the summary.
  const gate = wanted || r.s.waiting || (r.turn && r.live.find(i => i.kind === 'gate') ? { kind: r.live.find(i => i.kind === 'gate').gate } : null);
  const pane = r.task && gate ? paneOfGate(gate) : 'detail';
  go({ board: r.board, view: 'work', task: r.ref, pane }, { replace: replace || workIsSelected(r) });
  // The panel may not draw (the board does not list the task), and what it would have drawn is what follows the address.
  workTrack();
}

function selectWorkParent(key) {
  const p = workParentOf(key);
  if (!p) return;
  go({ board: workParentBoard(p), view: 'work', task: PARENT_REF + p.key, pane: 'detail' }, { replace: nav.task === PARENT_REF + p.key });
}

/* "Mark all read": every row that is new is read as of now (and `left`, so that 離れていた間に starts there too). */
function workReadAll() {
  const now = workServerNow();
  const byRepo = new Map();
  for (const j of workJudge().values()) {
    if (j.cls !== 'new') continue;
    byRepo.set(j.entry.nwo, [...(byRepo.get(j.entry.nwo) || []), [j.entry.id, { left: now, read: now }]]);
  }
  for (const [nwo, patches] of byRepo) workWriteMarks(nwo, patches);
}

/* The button beside a row: 既読にする (`left` too, as mark-all-read writes it, so 離れていた間に starts there). */
function workMarkRow(id, patch) {
  const e = work.entries.get(id);
  if (!e) return;
  workWriteMarks(e.nwo, [[id, patch]]);
}

/* The parks and un-parks this document shows that the marks have not seen (`workParkPatches`), written once per repository. */
function workTrackParks() {
  const entries = workEntries(work.doc, key => answeredGates.has(key));
  const now = workServerNow();
  for (const repo of workRepos()) {
    if (repo.error) continue;
    const mine = entries.filter(e => e.nwo === repo.nwo);
    const patches = workParkPatches(mine, workMarks(repo.nwo), now);
    if (patches.length) workWriteMarks(repo.nwo, patches);
  }
}

wk('work-view').addEventListener('click', e => {
  let b;
  if (e.target.closest('[data-wk-read-all]')) return workReadAll();
  if ((b = e.target.closest('[data-wk-read]'))) return workMarkRow(b.dataset.wkRead, { read: workServerNow(), left: workServerNow() });
  if ((b = e.target.closest('[data-wk-fold]'))) {
    const at = prefs.workFolded.indexOf(b.dataset.wkFold);
    if (at >= 0) prefs.workFolded.splice(at, 1); else prefs.workFolded.push(b.dataset.wkFold);
    savePrefs();
    return renderWorkList();
  }
  if ((b = e.target.closest('[data-wk-wake]'))) {
    const c = work.hubsByKey.get(b.dataset.wkWake);
    return c && !b.disabled && wakeHub(c.hub, c.wakeBase);
  }
  if ((b = e.target.closest('[data-wk-hub]'))) {
    const c = work.hubsByKey.get(b.dataset.wkHub);
    return c && selectWorkRow(c);
  }
  if ((b = e.target.closest('[data-wk-parent]'))) return selectWorkParent(b.dataset.wkParent);
  if ((b = e.target.closest('.wk-row[data-wk]'))) {
    const r = work.rowsByKey.get(b.dataset.wk);
    return r && selectWorkRow(r);
  }
  if ((b = e.target.closest('[data-wk-group]'))) {
    if (prefs.workGroup === b.dataset.wkGroup) return;
    prefs.workGroup = b.dataset.wkGroup;
    savePrefs();
    renderWorkList();
  }
});

wk('work-view').addEventListener('change', e => {
  if (!e.target.matches('[data-wk-advance]')) return;
  prefs.reviewNext = e.target.checked;
  savePrefs();
  // The other headers that have the box follow.
  renderWorkList();
});

/* A gate on the selected row was answered or closed (`gateAnswered`, decide.js): with 「処理したら次へ」 on, the next row of 新着 opens,
   by replacing the address, so that going back leaves the list and does not step through what was just answered. */
function workAdvanceAfter(key) {
  if (!prefs.reviewNext || view !== 'work' || !work.doc) return;
  const id = workSelectedId(workSelected());
  const entry = id && work.entries.get(id);
  // Only a gate of the row on screen: another row's was not what the person was working through.
  if (!entry || !entry.items.some(i => i.kind === 'gate' && i.key === key)) return;
  // Another gate still open on it: that is what the person goes on with, here.
  if (entry.items.some(i => i.kind === 'gate' && i.key !== key && !answeredGates.has(i.key))) return;
  const fresh = [...workJudge()].filter(([, j]) => j.cls === 'new').map(([k]) => k);
  const next = workNextNew(work.newOrder, id, fresh);
  if (!next) return;
  const { rows, turns } = workListRows();
  const row = [...rows, ...turns].find(r => r.id === next);
  if (row) selectWorkRow(row, { replace: true });
}

/* ── 離れていた間に (#556) ── */

/* A time as the timeline says it: `when` in the reader's time, `ago` as the hover. */
const workAwayAt = secs => `<span class="at" title="${esc(ago(secsStamp(secs)))}">${esc(when(secsStamp(secs)))}</span>`;

/* The block of 離れていた間に: the last thing the person did, when they left, what happened since, and what the task waits on
   now. `m` is `workAwayModel` or `workParentAway` (which has no `left`); `who` of a line is a child's title, for a parent. */
function workAwayHtml(m, now) {
  const li = (at, html, cls = '') => `<li${cls ? ` class="${cls}"` : ''}>${at}<div class="what">${html}</div></li>`;
  const who = ev => ev.title ? `<span class="wk-away-who">${esc(ev.title)}</span>` : '';
  const items = [];
  if (m.last) items.push(li(workAwayAt(m.last.at), `<div>${who(m.last)}最後にしたこと — ${esc(m.last.text)}</div>`, 'last'));
  if (m.left != null && now != null) items.push(li(workAwayAt(m.left), `<div>${esc(agoLabel(minutesSince(m.left, now)))}に離れた</div>`, 'left'));
  if (m.events.length) for (const ev of m.events) items.push(li(workAwayAt(ev.at), `<div>${who(ev)}${esc(workAwayText(ev))}</div>`));
  else items.push(li('<span class="at"></span>', '<div class="tp-muted">離れていた間の動きはありません</div>'));
  if (m.more) items.push(li('<span class="at"></span>', `<div class="tp-muted">${esc(`ほか ${m.more} 件`)}</div>`));
  if (m.now) items.push(li('<span class="at">いま</span>', `<div>${esc(m.now)}</div>`, 'now'));
  return `${secTitle('離れていた間に')}<ol class="timeline wk-away-list">${items.join('')}</ol>`;
}

/* The block above a task's or a session's タスクサマリ: filled into #tp-away, which the page keeps, and hidden unless the work
   view has a task or session the person has left or acted on. `sel` is { task, sess, all } (`all` the gates of the task, answered
   ones too), or null. */
function renderWorkAway(sel) {
  const el = tp('tp-away');
  if (!el) return;
  let html = '';
  const id = view === 'work' && sel ? workSelectedId(workSelected()) : null;
  const e = id ? work.entries.get(id) : null;
  if (e && !e.isHub && work.doc) {
    const m = workAwayModel(e, workMarks(e.nwo)[e.id], sel.all || [], workServerNow(),
      // What happens while the row is open is this visit's, not what was missed.
      work.open?.id === e.id ? work.open.at : Infinity);
    if (m) html = `<div class="m3-filled-card wk-away-card">${workAwayHtml(m, workServerNow())}</div>`;
  }
  if (el.dataset.sig === html) return;
  el.dataset.sig = html;
  el.innerHTML = html;
  el.hidden = !html;
}

/* The same for a parent, over its children, into the overview's #tp-parent-away. A child is the entry of a task whose parent
   this is; a merged child with no row is read from its PR's time and the mark its row had, which is pruned a week after
   it merges. Opening a parent writes no marks: each child is judged by its own. */
function renderParentAway(p) {
  const el = tp('tp-parent-away');
  if (!el) return;
  const marks = workMarks(p.repo.nwo);
  const entries = [...work.entries.values()].filter(e => e.nwo === p.repo.nwo && !e.isHub && e.task?.parent === p.key);
  const kids = entries.map(e => ({ title: e.task.title || e.ref, entry: e, mark: marks[e.id] }));
  for (const c of p.children || []) {
    if (c.progress !== 'merged' || entries.some(e => e.board === c.hub && e.task.id === c.id)) continue;
    kids.push({ title: c.title || `#${c.id}`, entry: null, mark: marks[`${c.hub}/${c.id}`], mergedAt: awaySecs(c.prTurnAt) });
  }
  const m = workParentAway(kids, workServerNow());
  const html = m ? `<div class="m3-filled-card wk-away-card">${workAwayHtml({ ...m, left: null }, workServerNow())}</div>` : '';
  if (el.dataset.sig !== html) {
    el.dataset.sig = html;
    el.innerHTML = html;
  }
  el.hidden = !html;
}

/* ── the parent in the panel ── */

/* The parent's overview, in the panel's place: how far it is, the stack, and what can be done to it. */
function parentOverviewHtml(p) {
  const segs = workSegments(p);
  const merged = p.merged || 0;
  const total = Math.max(p.total || 0, (p.children || []).length);
  const rowOf = c => (p.repo?.rows || []).find(r => r.board === c.hub && r.task?.id === c.id) || null;
  const stateOf = c => {
    const r = rowOf(c);
    return r ? sessionState(r.session, { now: work.doc?.now, repo: p.repo.nwo, hubs: p.repo.hubs || [], sessions: [] }) : null;
  };
  const inStack = c => !!c.on || (p.children || []).some(x => x.on === c.id && x.onHub === c.hub);
  const chain = p.stacked ? (p.children || []).filter(inStack) : [];
  const root = chain[0];
  const step = c => {
    const r = rowOf(c);
    const st = stateOf(c);
    const [cls, label] = WORK_PROGRESS[c.progress] || WORK_PROGRESS['not-started'];
    return `<li><span class="seg ${cls}" role="img" aria-label="${esc(label)}" title="${esc(label)}"></span>${st ? workGlyphHtml(st) : ''}<span>${esc(r?.task?.title || c.title || `#${c.id}`)}</span>${c.branch ? `<span class="base">${esc(c.branch)}</span>` : ''}</li>`;
  };
  const stack = chain.length
    ? `<div class="m3-filled-card">${secTitle('stack')}<ul class="wk-stack-list">${root?.base ? `<li><span class="base">${esc(root.base)} ←</span></li>` : ''}${chain.map(step).join('')}</ul></div>` : '';
  const served = boards.some(b => b.slug === p.hub);
  const nextWhy = served ? '' : 'この hub のボードはこのサーバーにありません';
  const canAdd = !!httpUrl(p.url);
  return `<div class="wk-overview">
    <div class="m3-filled-card">${secTitle('進み具合')}
      <div class="wk-ov-bar"><span class="wk-bar" role="img" aria-label="${esc(`${merged} / ${total} マージ`)}">${segs.map(x => `<i class="${x.cls}" title="${esc(x.title)}"></i>`).join('')}</span><span class="wk-ov-n">${merged} / ${total} マージ</span></div>
      <div class="tp-muted">${esc((p.children || []).length ? `このボードが知っている子 ${(p.children || []).length} 件` : '子タスクはまだありません')}</div>
    </div>
    ${stack}
    <div id="tp-parent-away" hidden></div>
    <div class="m3-filled-card">${secTitle('操作')}<div class="tp-gate-actions">
      <button type="button" class="btn-m3-tonal" data-wk-next${nextWhy ? ' disabled' : ''} title="${esc(nextWhy || 'adj send --kind next (着手を促す)')}"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">bolt</span><span>次を着手させる</span></button>
      ${canAdd ? `<button type="button" class="btn-m3-tonal" data-add-child="${esc(p.url)}" data-add-child-board="${esc(p.hub)}"><span class="material-symbols-outlined" style="font-size:16px;" aria-hidden="true">add</span><span>子タスクを足す</span></button>` : ''}
    </div></div>
  </div>`;
}

function parentHeadHtml(p) {
  const link = httpUrl(p.url);
  return `<div class="tp-head-main">
      <div class="tp-badges"><span class="tp-key">親 Issue</span>${p.repo ? `<span class="origin-chip" title="${esc(p.repo.nwo)}"><span class="material-symbols-outlined" aria-hidden="true">folder</span><span>${esc(workRepoName(p.repo.nwo))}</span></span>` : ''}</div>
      <h2 class="tp-title">${esc(`${p.number ? `#${p.number} ` : ''}${p.title || p.key}`)}</h2>
    </div>
    ${panelBtnsHtml(link ? `<a class="btn-m3-text tp-jump" href="${esc(link)}" target="_blank" rel="noopener noreferrer" title="GitHub で開く">GitHub</a>` : '')}`;
}
const workRepoName = nwo => (nwo || '').split('/').pop() || nwo;

/* The panel when it holds no task, hub or session of its board: a parent's overview, or a line saying the board the
   row opened on does not have what the list showed. */
function renderWorkPanelOnly(parent) {
  tp('tp-tabs').hidden = true;
  tp('tp-detail').hidden = false;
  tp('tp-term').hidden = true;
  panelShown = null;
  setPanelPart('head', tp('tp-head'), parent ? parentHeadHtml(parent)
    : `<div class="tp-head-main"><h2 class="tp-title">この仕事</h2></div>${panelBtnsHtml('')}`);
  setPanelPart('tabs', tp('tp-tabs'), '');
  setPanelPart('links', tp('tp-links'), '');
  setPanelPart('gate', tp('tp-gate'), '');
  setPanelPart('rest', tp('tp-rest'), parent ? parentOverviewHtml(parent)
    : `<div class="m3-filled-card"><div class="tp-muted">${isGateRef(selectedTaskId) ? 'この確認待ちは、もう片付いています' : 'この仕事は、いまの一覧にありません'}</div></div>`);
  if (parent) renderParentAway(parent);
  renderHandForm(null);
  tp('tp-form').hidden = true;
}

/* The panel's clicks when it holds no task of its own board: a parent's buttons. */
function workPanelClick(e) {
  const hit = sel => e.target.closest(sel);
  let b;
  if (hit('[data-tp-close]')) return closeTaskPanel();
  const p = parentOfRef(selectedTaskId);
  if (!p) return;
  if ((b = hit('[data-wk-next]'))) return b.disabled ? undefined : nudgeHub(`/b/${p.hub}`);
  if ((b = hit('[data-add-child]'))) return openChildForm(b.dataset.addChild, b.dataset.addChildBoard);
}

/* ── widths ── */

/* A width the person drags from an edge, or moves with the arrow keys, or puts back with a double click. `at` is the
   width the pointer's x asks for; `apply` sets it. Saved when the pointer is let go. */
function bindWidthHandle(handle, { read, apply: set, min, max, reset, at }) {
  // A separator that can be focused says where it stands.
  const apply = w => { set(w); handle.setAttribute('aria-valuenow', String(w)); };
  handle.setAttribute('aria-valuemin', String(min));
  handle.setAttribute('aria-valuemax', String(max()));
  handle.setAttribute('aria-valuenow', String(read()));
  const clamp = w => Math.round(Math.max(min, Math.min(max(), w)));
  handle.addEventListener('pointerdown', e => {
    if (matchMedia('(max-width: 720px)').matches) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    document.body.classList.add('wk-dragging');
    const move = ev => apply(clamp(at(ev.clientX)));
    const end = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
      document.body.classList.remove('wk-dragging');
      savePrefs();
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  });
  handle.addEventListener('keydown', e => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    apply(clamp(read() + (e.key === 'ArrowRight' ? 24 : -24)));
    savePrefs();
  });
  handle.addEventListener('dblclick', () => {
    apply(reset);
    savePrefs();
  });
}

bindWidthHandle(wk('wk-list-resize'), {
  read: () => prefs.workListWidth, min: 260, max: () => 640, reset: 380,
  at: x => x - wk('work-view').getBoundingClientRect().left,
  apply: w => { prefs.workListWidth = w; document.body.style.setProperty('--wk-list-w', `${w}px`); },
});
bindWidthHandle(wk('rail-resize'), {
  read: () => prefs.railWidth, min: 180, max: () => 400, reset: 240,
  at: x => x,
  apply: w => { prefs.railWidth = w; document.body.style.setProperty('--rail-w-set', `${w}px`); },
});

registerView('work-view', { render: () => renderWorkView() });
