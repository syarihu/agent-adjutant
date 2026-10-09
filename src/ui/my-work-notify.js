/* ── Desktop notifications of 「いまの仕事」: when to ring and what to say ───────────────────────────
   Pure, like my-work-seen.js: these read the entries and the settings they are handed and touch neither the
   page, `Notification` nor local storage, so `src/ui/tests/my-work-notify.test.js` runs them under
   `node --test`. Loaded after my-work-seen.js (the entries) and before my-work.js, which rings. */

/* The kinds the person can choose, and what each is called in a notification's first line. */
const NOTIFY_STATE_WORD = { gate: '確認待ち', permission: '入力待ち', done: '完了', failed: '失敗' };
const NOTIFY_BODY_LINES = 3;
const NOTIFY_BODY_CHARS = 200;

/* What the browser's storage held, read as the settings: a kind that is not a boolean is the default. The defaults
   are in the function, not in constants: core.js reads the prefs while it loads, before this file's constants exist. */
function notifyPrefs(raw) {
  const defaults = { waiting: true, done: false, failed: true };
  const src = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {};
  const out = {};
  for (const k of Object.keys(defaults)) out[k] = typeof src[k] === 'boolean' ? src[k] : defaults[k];
  return out;
}

/* Whether this page rings desktop notifications for waits and gates at all: the browser allows them and the person
   left 確認待ち on. The server is told on every request, and rings its own `notification` instead once no page has said true for about 90 seconds. */
function notifyPageRings(permission, prefs) {
  return permission === 'granted' && !!prefs?.waiting;
}

/* Whether nothing rings for the entry: a task set aside on purpose, or the row the person is looking at. Open in a
   hidden tab it is not looked at, so it rings. */
function notifyQuiet(entry, openId, visible) {
  if (parkOf(entry?.task)) return true;
  return !!visible && openId != null && entry?.id === openId;
}

/* Whether a queued gate or wait (`queuedAt`, ms) is to be rung now whatever the document says: it was queued before the
   request that gave the latest document began (`fetchedAt`, ms; null when no document was just read), or it has
   waited `maxMs`. A younger one may only be missing from a document that was already on its way. */
function notifyPendingFinal(queuedAt, fetchedAt, now, maxMs) {
  return (fetchedAt != null && queuedAt < fetchedAt) || now - queuedAt >= maxMs;
}

/* Whether a work entry is the one a gate (`slug`, `id`, and the task it names, when it names one) is about. */
function notifyGateMatch(entry, slug, id, task) {
  return entry.gates.some(x => x.key === `${slug}/${id}`) || (!!task && entry.board === slug && entry.ref === task);
}

/* Whether a work entry is the one a wait is about: on its board, the session that waits, or another session of the
   same ledger row (the entry shows only the better of a task's sessions). */
function notifyWaitMatch(entry, slug, sessionId, agentSessionId) {
  if (entry.board !== slug) return false;
  const s = entry.session;
  return s?.id === sessionId || (!!agentSessionId && s?.agentSession?.sessionId === agentSessionId);
}

/* The end the session's ledger row shows, as workItems would list it, whether or not a gate hides it or the session is
   present at the moment: a flapping session must not ring an old end when it comes back. */
function notifyLedgerEnd(e) {
  const a = e.session?.agentSession;
  if (!a || a.error || (a.status !== 'done' && a.status !== 'failed') || !Number.isFinite(a.updatedAt)) return null;
  return { kind: a.status, since: a.updatedAt };
}

/* The ends that are new since `state` (what the last call returned, or null on the first): `{events, seen, learned}`.
   `okRepos` are the repositories (nwo) this document read without an error. A repository is learned on its first
   error-free appearance: until then its ends only go into the seen set, so one that was missing from the first
   document does not ring its whole history when it is read. The seen set only grows, whatever the settings say, and
   holds a key while its entry is out of the document for a poll; only the events are filtered by the settings, so
   turning a kind on does not ring for what was already there. The ledger's own end is learned too when a gate hides
   it as an item: when the gate is answered the item appears with the old `since`, and that turn ended long ago. An
   end with no `since` is not told apart from the next, so it is neither learned nor rung. One event to an entry at
   a time, a failure before a completion. */
function notifyEndEvents(entries, state, { prefs, openId, visible, okRepos }) {
  const seen = new Set(state?.seen || []);
  const learnedBefore = state?.learned || new Set();
  const ok = new Set(okRepos || []);
  const learned = new Set([...learnedBefore, ...ok]);
  const events = [];
  for (const e of entries || []) {
    const listening = ok.has(e.nwo) && learnedBefore.has(e.nwo);
    const fresh = [];
    for (const kind of ['failed', 'done']) {
      const item = e.items.find(i => i.kind === kind);
      if (!item || !Number.isFinite(item.since)) continue;
      const key = `${e.id}/${kind}/${item.since}`;
      const isNew = !seen.has(key);
      seen.add(key);
      if (isNew && listening) fresh.push({ kind, entry: e, key });
    }
    // After the items: a key the entry lists as an item is new, one it hides is only learned.
    const ledger = notifyLedgerEnd(e);
    if (ledger) seen.add(`${e.id}/${ledger.kind}/${ledger.since}`);
    const first = fresh.find(ev => prefs[ev.kind]);
    if (first && !notifyQuiet(e, openId, visible)) events.push(first);
  }
  return { events, seen, learned };
}

/* What the agent said, cut to a few lines and a few characters so a notification stays one. */
function notifyTrimMessage(text) {
  const lines = String(text || '').split('\n').map(l => l.trim()).filter(Boolean);
  let out = lines.slice(0, NOTIFY_BODY_LINES).join('\n');
  let cut = lines.length > NOTIFY_BODY_LINES;
  if (out.length > NOTIFY_BODY_CHARS) { out = out.slice(0, NOTIFY_BODY_CHARS); cut = true; }
  return cut ? `${out.trimEnd()}…` : out;
}

/* What the agent said at the end of the turn that ended, as the away block reads it: a message older than the status
   (by more than the slack) is from an earlier turn. Uses my-work-away.js, which loads after this file and is only
   called into at run time. */
function notifyLastMessage(a) {
  if (!a?.lastMessage) return '';
  const said = awaySecs(a.lastMessageAt);
  const at = awaySecs(a.updatedAt);
  return said == null || said >= (at ?? -Infinity) - WORK_AWAY_MESSAGE_SLACK ? a.lastMessage : '';
}

/* A notification's title and body. `kind` is gate, permission, done or failed. The first line of the body says
   what happened and where (state, repository, branch), as a subtitle would; the rest is the request, the gate's
   title, or what the agent last said. `gate` is the gate's title, `label` its kind's name, and `request` the
   permission request, for where the entry does not hold them. */
function notifyContent(kind, entry, { gate = '', request = '', label = '' } = {}) {
  const s = entry?.session;
  const title = entry?.task?.title || s?.title || s?.name || entry?.gateTitle || gate || '';
  const repo = (entry?.nwo || '').split('/').pop();
  const state = kind === 'gate' && label ? `${NOTIFY_STATE_WORD.gate}（${label}）` : NOTIFY_STATE_WORD[kind];
  const head = [state, repo, s?.branch].filter(Boolean).join(' · ');
  const rest = kind === 'gate' ? gate
    : kind === 'permission' ? request
      : notifyTrimMessage(notifyLastMessage(s?.agentSession));
  return { title, body: rest ? `${head}\n${rest}` : head };
}
