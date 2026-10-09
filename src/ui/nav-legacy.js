/* ── Old addresses: where a link made before #558 lands now ─────────────────────────────────────
   The セッション tab and 要対応 are gone: 「いまの仕事」 has both. Their links still work and are
   redirected here, as pure data, so `src/ui/tests/nav-legacy.test.js` runs this under `node --test`. Loaded
   before nav.js, which reads the address through it (`parseUrl`, `boot`) and writes the new spelling back.

     /review                                  resident: 「いまの仕事」, every board; a board served alone: its エージェント board
     /review?item=<board>/<gate>              resident: the same, with that gate opened once the work document is in (`pendingGate`);
     /b/<board>/#gate/<gate>                  a board served alone: the gate in its panel (`task=gate:<gate>`)
     ?view=sessions&task=<ref>&pane=<tab>     resident: 「いまの仕事」 on that board and task (the terminal is in the middle now, so
     ?view=sessions&session=<id>              `pane=term` is the summary); with no task, or on `/`, every board; alone: エージェント
     /b/<board>/#session/<id>, #sessions

   An item that is a permission wait (`<board>/wait:<id>:<since>`) or a bare id has no gate to open: the list only. */

/* `text` decoded, or null when it has a malformed escape: a hand-edited link must not stop the page from loading. */
function legacySafeDecode(text) {
  try { return decodeURIComponent(text); } catch { return null; }
}

/* `legacyNav({ pathname, search, hash }, multiBoard)`: null for an address that is not an old one, else
   `{ nav, pendingGate }` where `nav` is what to lay over what the address says (board, view, task, pane) and `pendingGate`
   is `<board>/<gate>` to open once the work document says where it is, or null. */
function legacyNav(loc, multiBoard) {
  const path = (loc && loc.pathname) || '/';
  const q = new URLSearchParams((loc && loc.search) || '');
  const hash = (loc && loc.hash) || '';
  const board = /^\/b\/([^/]+)/.exec(path);
  const slug = board ? board[1] : null;
  const list = { board: 'all', view: 'work', task: null, pane: 'detail' };

  const isReview = path === '/review';
  const gateHash = /^#gate\/(.+)$/.exec(hash);
  const sessHash = /^#sessions?(?:\/(.+))?$/.exec(hash);
  const sessView = q.get('view') === 'sessions';
  if (!isReview && !gateHash && !sessHash && !sessView) return null;

  // A gate's id as the old link spelled it: `<board>/<id>` in the queue, a bare `<id>` on a board's own page.
  let gate = null;
  if (isReview) {
    const item = q.get('item') || '';
    if (item && !/(^|\/)wait:/.test(item)) gate = item;
  } else if (gateHash) {
    const id = legacySafeDecode(gateHash[1]);
    if (id) gate = id;
  }

  if (!multiBoard) {
    // A board served alone has no list of work and never had the セッション tab: the queue's gate is in its panel.
    if (!gate) return { nav: { view: 'agent' }, pendingGate: null };
    const id = gate.slice(gate.lastIndexOf('/') + 1);
    return { nav: id ? { view: 'agent', task: `gate:${id}`, pane: 'detail' } : { view: 'agent' }, pendingGate: null };
  }

  if (isReview || gateHash) {
    // A gate with a board in its name; a bare id on a board's own page is that board's.
    const ref = gate && gate.includes('/') ? gate : gate && slug ? `${slug}/${gate}` : null;
    return { nav: list, pendingGate: ref };
  }

  // The セッション tab: a task or a session of one board, else the list.
  let task = q.get('task');
  let fromSession = false;
  const old = q.get('session') || (sessHash && sessHash[1] ? legacySafeDecode(sessHash[1]) : null);
  if (!task && old) {
    task = `session:${old}`;
    fromSession = true;
  }
  if (!slug || !task) return { nav: list, pendingGate: null };
  // The terminal has no tab of its own now: it is in the middle, whichever tab the panel is on.
  const pane = fromSession ? 'detail' : ['review', 'check', 'history'].includes(q.get('pane')) ? q.get('pane') : 'detail';
  return { nav: { board: slug, view: 'work', task, pane }, pendingGate: null };
}
