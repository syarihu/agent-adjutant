/* ── いまの仕事 「AI 下読み」: the PRs others asked you to review, as rows ───────────────────────────────
   Pure, like my-work-seen.js: these read a record of `GET /api/others` (docs/architecture.md, "PRs others
   asked you to review") and the read marks they are handed and return, touching neither the page nor local
   storage, so `src/ui/tests/my-work-others.test.js` runs them under `node --test`. Loaded after
   my-work-seen.js (the marks) and before my-work-notify.js; my-work-others-view.js draws and keeps the marks.

   新着 here is judged per event. A record says when each thing happened to it (`events`: a request, a
   re-request, a finished AI 下読み, a push since your review), and a row is new when one of them is later than
   when the person last read it. They read it by pressing 既読 (a mark, `read`, in seconds on the server's clock) or
   by acting on it (the AI 下読み they started: `ai.startedAt`). Selecting the row reads nothing. A PR waiting on
   its author, or done, has nothing for the person and is never new. */

/* The state of a record: Material Symbols name, the words, and the class the stylesheet colours it by. A state this
   page does not know is shown as it came, never as a request: a row that asks for nothing must not look like it does. */
const OTHERS_STATE = {
  requested: ['rate_review', 'レビュー依頼', 'ov-requested'],
  'ai-reading': ['progress_activity', 'AI 下読み中', 'ov-reading'],
  'ai-ready': ['menu_book', '下読み済み', 'ov-ready'],
  pushed: ['update', 'push された', 'ov-pushed'],
  'waiting-on-author': ['hourglass_top', '作者の対応待ち', 'ov-dim'],
  done: ['check_circle', '終わった', 'ov-dim'],
};
function othersStateOf(rec) {
  return OTHERS_STATE[rec?.state] || ['help', rec?.state ? String(rec.state) : '状態不明', 'ov-dim'];
}

/* A list not synced for more than this many seconds is called stale. */
const OTHERS_STALE_SECS = 1800;
/* Additions plus deletions: below the first bound is S, below the second M, and the rest L. */
const OTHERS_SIZE = [[200, 'S'], [1000, 'M']];
/* How long a reader may stay running, in seconds, before a new request takes its read over (the server's HUNG_READER_SECS). */
const OTHERS_HUNG_SECS = 1800;
/* How many AI 下読み the code-review hub runs at once, when the hub does not say. */
const OTHERS_AI_MAX = 3;
/* Where this browser keeps its read marks: record id to { read }, in server seconds. */
const OTHERS_SEEN_KEY = 'adj.seenOthers';

const OTHERS_EVENTS = ['requested', 'rerequested', 'aiReady', 'pushed'];
/* The states in which the PR is waiting on the person. */
const OTHERS_LIVE = ['requested', 'ai-reading', 'ai-ready', 'pushed'];

/* What the record has for the person, with when each began: its events, those that have a time. None for a PR that waits
   on its author or is done, and for a state this page does not know. */
function othersItems(rec) {
  if (!OTHERS_LIVE.includes(rec?.state)) return [];
  const out = [];
  for (const kind of OTHERS_EVENTS) {
    const since = stampSecs(rec.events?.[kind]);
    if (since != null) out.push({ kind, since });
  }
  return out;
}

/* When the person last acted on the record other than by pressing 既読: they started its AI 下読み. -Infinity when never.
   (A later issue adds the time they put drafts into a pending review; it is read here.) */
function othersActedAt(rec) {
  const times = [stampSecs(rec?.ai?.startedAt)];
  return Math.max(-Infinity, ...times.filter(x => Number.isFinite(x)));
}

/* 'new' or null: an event later than both the mark and the last action. A record with no event time is never new. */
function othersSeenClass(rec, mark = {}) {
  const items = othersItems(rec);
  if (!items.length) return null;
  const read = Math.max(Number.isFinite(mark.read) ? mark.read : -Infinity, othersActedAt(rec));
  return items.some(i => i.since > read) ? 'new' : null;
}

/* The ids of the new records among `records`, judged by `marks` (id to mark). Not narrowed by any filter. */
function othersNewIds(records, marks) {
  return (records || []).filter(r => othersSeenClass(r, marks?.[r.id] || {}) === 'new').map(r => r.id);
}

/* S, M or L, by additions plus deletions. */
function othersSize(rec) {
  const n = (Number(rec?.additions) || 0) + (Number(rec?.deletions) || 0);
  for (const [below, label] of OTHERS_SIZE) if (n < below) return label;
  return 'L';
}

/* How long ago the last sync was, in seconds, as of `nowEst` (null when there was none). */
function othersSyncAge(lastSync, nowEst) {
  const at = stampSecs(lastSync?.at);
  return at == null ? null : nowEst - at;
}

/* Whether the last sync is more than 30 minutes old. A list never synced is not stale: it says so in its own words. */
function othersStale(lastSync, nowEst) {
  const age = othersSyncAge(lastSync, nowEst);
  return age != null && age > OTHERS_STALE_SECS;
}

/* Whole minutes since the last sync (0 when there was none). */
function othersStaleMinutes(lastSync, nowEst) {
  const age = othersSyncAge(lastSync, nowEst);
  return age == null ? 0 : Math.max(0, Math.floor(age / 60));
}

/* `依頼 5分前`, or nothing when the record has no request time. */
function othersRequestedLabel(rec, nowSecs) {
  const at = stampSecs(rec?.requestedAt);
  return at == null ? '' : `依頼 ${agoLabel(minutesSince(at, nowSecs))}`;
}

function othersRepoKey(rec) {
  return String(rec?.repo || '').toLowerCase();
}

/* The repository filter that applies: `filter` (lower case; '' is すべて), or すべて when no record is in that repository
   (the chip is not kept for a repository that has dropped out, and nothing is saved by this). */
function othersEffectiveRepo(records, filter) {
  return filter && (records || []).some(r => othersRepoKey(r) === filter) ? filter : '';
}

/* A row's place in its group: what is done last, then the oldest request first (none last), then by number. */
function othersRowOrder(a, b) {
  const done = x => x.state === 'done' ? 1 : 0;
  if (done(a) !== done(b)) return done(a) - done(b);
  if ((a.requestedAt == null) !== (b.requestedAt == null)) return a.requestedAt == null ? 1 : -1;
  if (a.requestedAt !== b.requestedAt && a.requestedAt != null) return a.requestedAt < b.requestedAt ? -1 : 1;
  return (a.number || 0) - (b.number || 0);
}

/* The records as groups, one per repository in name order with case ignored: `{key, label, unregistered, rows}`. The label is
   the repository as GitHub spells it, and `unregistered` is set when adj has no board for it. `repoFilter` narrows to
   one repository (lower case), falling back to all of them when it names none that is here. */
function othersGroups(records, repoFilter = '') {
  const only = othersEffectiveRepo(records, repoFilter);
  const byKey = new Map();
  for (const r of records || []) {
    const key = othersRepoKey(r);
    if (only && key !== only) continue;
    if (!byKey.has(key)) byKey.set(key, { key, label: r.repo, unregistered: false, rows: [] });
    const g = byKey.get(key);
    if (r.registered === false) g.unregistered = true;
    g.rows.push(r);
  }
  const groups = [...byKey.values()].sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
  for (const g of groups) g.rows.sort(othersRowOrder);
  return groups;
}

/* The chips that narrow the list: one per repository, in name order, with how many records it has. */
function othersRepoChips(records) {
  return othersGroups(records, '').map(g => ({ key: g.key, label: g.label, count: g.rows.length }));
}

const OTHERS_DONE_WORD = { approved: '承認しました', merged: 'マージされました', closed: '閉じられました', withdrawn: '依頼が取り下げられました' };

/* Where a queued read stands: its 1-based place among the records waiting, by when it was asked, then id. 0 when `records`
   does not hold it (then no number is said). */
function othersQueuePlace(rec, records) {
  const queued = (records || []).filter(r => r.ai?.status === 'queued')
    .sort((a, b) => String(a.ai.startedAt || '') < String(b.ai.startedAt || '') ? -1 : String(a.ai.startedAt || '') > String(b.ai.startedAt || '') ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  return queued.findIndex(r => r.id === rec?.id) + 1;
}

/* What the AI 下読み of a record says, and the one button it offers: `{kind, text, action}`. `kind` is none, queued, running,
   done, stale or failed; `action` is the label of the button (or null), and the button is offered only on a PR that waits on
   the person. A read running for longer than 30 minutes (`nowSecs` says when it is) is a reader that has hung: it is offered
   the same button, since asking again takes it over. A status this page does not know says nothing and offers nothing. */
function othersAiView(rec, records, nowSecs) {
  const ai = rec?.ai;
  const live = ['requested', 'ai-ready', 'pushed'].includes(rec?.state);
  if (!ai) return { kind: 'none', text: '', action: live && rec.state !== 'ai-ready' ? 'AI 下読み' : null };
  switch (ai.status) {
    case 'queued': {
      const place = othersQueuePlace(rec, records);
      return { kind: 'queued', text: place ? `AI 下読みの順番待ち（${place} 番目）` : 'AI 下読みの順番待ち', action: null };
    }
    case 'running': {
      const launched = stampSecs(ai.launchedAt);
      if (Number.isFinite(nowSecs) && launched != null && nowSecs - launched > OTHERS_HUNG_SECS) {
        return { kind: 'hung', text: 'AI 下読み中（30 分以上たっています）', action: live || rec.state === 'ai-reading' ? 'もう一度 AI 下読み' : null };
      }
      return { kind: 'running', text: 'AI 下読み中', action: null };
    }
    case 'ready': {
      if (ai.stale) {
        const sha = ai.sha ? String(ai.sha).slice(0, 7) : '';
        return { kind: 'stale', text: `下読み済み（前の push 時点${sha ? ` ${sha}` : ''}）`, action: live ? '下読みし直す' : null };
      }
      const c = ai.counts;
      const parts = [['要注意', c?.attention], ['確認', c?.check], ['下書き', c?.drafts]].filter(([, n]) => Number.isFinite(n)).map(([w, n]) => `${w} ${n}`);
      return { kind: 'done', text: ['下読み済み', ...parts].join(' · '), action: null };
    }
    case 'failed':
      return { kind: 'failed', text: `AI 下読みに失敗しました${ai.error ? `: ${ai.error}` : ''}`, action: live ? 'もう一度 AI 下読み' : null };
    default:
      return { kind: 'none', text: '', action: null };
  }
}

/* The line a row says about its state: `{cls, text}`, `cls` 'dim' for what asks nothing of the person. `records` gives a
   queued read its place. */
function othersStateLine(rec, records, nowSecs) {
  const ai = othersAiView(rec, records, nowSecs);
  const withAi = text => ai.text ? `${text} · ${ai.text}` : text;
  const rerequest = rec?.rerequest ? 'レビューを頼み直されました' : '';
  switch (rec?.state) {
    case 'requested':
      return { cls: '', text: withAi(`あなたのレビュー待ち${rerequest ? ` · ${rerequest}` : ''}`) };
    case 'ai-reading':
      return { cls: '', text: ai.text || 'AI 下読み中' };
    case 'ai-ready':
      return { cls: '', text: ai.text || '下読み済み' };
    case 'pushed': {
      const n = rec.commitsSinceReview;
      const text = Number.isFinite(n) ? `前回のレビューから +${n} コミット` : '前回のレビューのあとに push されました';
      return { cls: '', text: withAi(rerequest ? `${text} · ${rerequest}` : text) };
    }
    case 'waiting-on-author': {
      const changes = rec.myReview?.state === 'changes-requested';
      return { cls: 'dim', text: `作者の対応待ち — ${changes ? '変更を依頼しました' : 'コメントしました'}` };
    }
    case 'done': {
      const word = OTHERS_DONE_WORD[rec.doneReason];
      return { cls: 'dim', text: `終わった${word ? ` — ${word}` : ''} · 1日で消えます` };
    }
    default:
      return { cls: 'dim', text: String(rec?.state || '') };
  }
}

/* The row's own 「分割案」, for a PR that is out of review. Nothing yet: a later issue says when. */
function othersOutOfReview(rec) {
  return null;
}

/* The code-review hub's row: `hub` is the `hub` of the document (absent from an older server). An absent or stopped hub is
   stopped, and says that AI 下読み is unavailable while sync and the list still work. `running`, `queued`, `max` and `startable`
   are the hub's; when it does not say how many are running, they are counted from the records. `canRead` is whether a read can
   be asked for now: the hub is running and a reader can be opened (the server says; an older one that does not is taken to
   mean that a running hub can). When it cannot, `why` says what is missing, and is the title of the buttons. */
function othersHubView(hub, records) {
  const counted = status => (records || []).filter(r => r.ai?.status === status).length;
  const max = Number.isFinite(hub?.max) && hub.max > 0 ? hub.max : OTHERS_AI_MAX;
  const stopped = !hub || !hub.state || hub.state === 'stopped';
  const canRead = !stopped && (typeof hub.canRead === 'boolean' ? hub.canRead : true);
  const why = !hub ? 'コードレビュー hub はまだありません。AI 下読みは使えません（同期と一覧はそのまま使えます）'
    : stopped ? 'hub が止まっています。AI 下読みは使えません（同期と一覧はそのまま使えます）'
    : !canRead ? 'このターミナルでは読み手を開けません。AI 下読みには terminal.preset の tmux が必要です（同期と一覧はそのまま使えます）' : '';
  return {
    state: stopped ? 'stopped' : String(hub.state),
    running: Number.isFinite(hub?.running) ? hub.running : counted('running'),
    queued: Number.isFinite(hub?.queued) ? hub.queued : counted('queued'),
    max, startable: stopped && !!hub?.startable, canRead, why,
  };
}

/* What a redraw of the list depends on, from the document: the fields the rows show, the last sync, the hub, and the minute
   (times are shown to the minute), so a poll that changes none of them draws nothing. */
function othersSig(doc, nowEst) {
  const recs = (doc?.records || []).map(r => [r.id, r.title, r.author, r.repo, r.registered, r.state, r.doneReason, r.rerequest, r.additions, r.deletions,
    r.changedFiles, r.requestedAt, r.commitsSinceReview, r.myReview?.state, r.events, r.ai, r.ai?.stale, r.url]);
  const ls = doc?.lastSync;
  return JSON.stringify([recs, ls ? [ls.at, ls.complete, ls.truncated, (ls.failed || []).length, ls.error, ls.errorAt] : null, doc?.hub ?? null,
    Math.floor(nowEst / 60), othersStale(ls, nowEst)]);
}
