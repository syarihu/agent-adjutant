/* ── The organisation the page is looking at: one GitHub owner. #728 sets it; the other views read it. ── */

const ORG_EVENT = 'adj:org';
/* An owner-less repository (an nwo with no slash) is the organisation whose login is ''. It is saved as this, because
   '' in the prefs means nothing chosen; no GitHub login can be spelled so. */
const ORG_NONE_KEY = '-';
prefs.org = typeof prefs.org === 'string' ? prefs.org.toLowerCase() : '';
prefs.workRepo = typeof prefs.workRepo === 'string' ? prefs.workRepo.toLowerCase() : '';

/* The owner of a repository ('owner/name'), lowercase: GitHub spells logins case-insensitively. '' when there is none. */
const orgOf = nwo => { const s = String(nwo || ''), i = s.indexOf('/'); return i > 0 ? s.slice(0, i).toLowerCase() : ''; };

/* The organisations of the registered repositories, by name (the owner-less one last): { login, name, repos: [nwo…] }.
   A repository is registered when a board of it is open (not finished) or the work document lists it; the first
   spelling seen names the organisation and the repository. */
function orgList(boardList, doc) {
  const byLogin = new Map();
  const add = nwo => {
    if (!nwo) return;
    const login = orgOf(nwo);
    if (!byLogin.has(login)) byLogin.set(login, { login, name: login ? String(nwo).slice(0, String(nwo).indexOf('/')) : 'オーナーなし', repos: [] });
    const org = byLogin.get(login);
    if (!org.repos.some(r => r.toLowerCase() === String(nwo).toLowerCase())) org.repos.push(String(nwo));
  };
  for (const b of boardList || []) if (!b.finished) add(b.nwo);
  for (const r of doc?.repos || []) add(r.nwo);
  const byName = (a, b) => (a.toLowerCase() < b.toLowerCase() ? -1 : a.toLowerCase() > b.toLowerCase() ? 1 : 0);
  return [...byLogin.values()]
    .map(o => ({ ...o, repos: [...o.repos].sort(byName) }))
    .sort((a, b) => (!a.login) - (!b.login) || byName(a.login, b.login));
}

/* How many things wait on the person per organisation, as the TODO counts them: `items` is what the TODO gathered over every
   organisation (`todoItems`, todo.js: the gates to answer), each counted in the organisation of its
   repository, when that organisation is one of `orgs` (`orgList`). The result is login -> n. The badge and the menu read this, so what the TODO lists after switching is what they said. */
function orgWaitCounts(items, orgs) {
  const counts = new Map();
  // Only the organisations that can be picked: a count the menu has no entry for would point at nothing.
  const add = nwo => { const login = orgOf(nwo); if (orgs.some(o => o.login === login)) counts.set(login, (counts.get(login) || 0) + 1); };
  for (const w of [...(items?.quick || []), ...(items?.read || [])]) add(w.nwo);
  return counts;
}

/* What the work document says of an owner: { login, avatarUrl?, kind? }, or null while it is not listed. */
function orgIcon(login) {
  const want = String(login || '').toLowerCase();
  return (work.doc?.owners || []).find(o => String(o.login).toLowerCase() === want) || null;
}

/* The selected organisation's login (lowercase), or null while no repository is known: the one saved if it is still
   registered, else the one of the board in the address, else the first by name. Worked out each time, so a repository
   that is unregistered moves the selection without a pref being written. */
function currentOrg() {
  const orgs = orgList(boards, work.doc);
  if (!orgs.length) return null;
  const saved = prefs.org === ORG_NONE_KEY ? '' : prefs.org;
  if (prefs.org && orgs.some(o => o.login === saved)) return saved;
  const board = boards.find(b => b.slug === nav.board);
  if (board && board.nwo && orgs.some(o => o.login === orgOf(board.nwo))) return orgOf(board.nwo);
  return orgs[0].login;
}

/* Select `login`: saved, and told to the listeners (document event `adj:org`, detail { org, prev }). Nothing is told when
   it is the one already selected, and a login no repository has is refused. */
function setCurrentOrg(login) {
  const next = String(login ?? '').toLowerCase();
  if (!orgList(boards, work.doc).some(o => o.login === next)) return;
  const prev = currentOrg();
  const key = next || ORG_NONE_KEY;
  if (prefs.org !== key) { prefs.org = key; savePrefs(); }
  if (prev !== next) document.dispatchEvent(new CustomEvent(ORG_EVENT, { detail: { org: next, prev } }));
}

/* Call `fn(org, prev)` whenever another organisation is selected. */
function onOrgChange(fn) { document.addEventListener(ORG_EVENT, e => fn(e.detail.org, e.detail.prev)); }

/* The organisation selected and the ones that can be picked, worked out once for a pass over many repositories. */
function orgScope() {
  return { current: currentOrg(), known: new Set(orgList(boards, work.doc).map(o => o.login)) };
}

/* Whether a repository is in `scope`'s organisation, or in none the page lists (a review requested of the person in a repository
   that is not registered here): that one is shown under every selection, as nothing could select it. */
const orgInScope = (scope, nwo) => scope.current == null || !scope.known.has(orgOf(nwo)) || orgOf(nwo) === scope.current;

/* Whether a repository ('owner/name') belongs to the selected organisation. */
const inCurrentOrg = nwo => !!nwo && orgOf(nwo) === currentOrg();
