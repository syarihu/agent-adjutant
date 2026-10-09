/* ── いまの仕事: what happened while the person was away (#556) ─────────────────────────────────
   Pure, like my-work-seen.js: these read an entry of the work document (`workEntries`) and the read mark of its
   row, and return data, touching neither the page nor local storage, so `src/ui/tests/my-work-away.test.js`
   runs them under `node --test`. Loaded after my-work-seen.js (`workActedAt`) and before my-work.js, which draws.

   "Away" starts at the later of the time the person left the row (`mark.left`) and the time they last acted on it,
   the same `read` that makes a row new or looked at. What counts is what the work document itself shows, so the
   count on a row and the timeline in the panel cannot disagree: phase moves, gates opened, a permission wait, a
   turn that ended or failed, and the PR's turn changing. Not events: records, answers (they are the person's own
   acts, and are the last action), sub-agents and other fine-grained activity. */

/* What a PR's turn says in 新着 (my-work-seen.js has the words of 後で見る). */
const WORK_PR_NOW = {
  changes: '修正の依頼が来ています',
  'ci-failed': 'CI が落ちています',
  merge: 'マージできます',
  closed: 'マージされずに閉じられました',
};
/* What the PR did, by the turn it moved to (`PrTurn`, kebab-case) or, for a finished task, by its state. */
const WORK_PR_MOVED = {
  draft: 'PR が下書きになった',
  unrequested: 'PR がレビュー待ちになった',
  'other-reviewer': 'PR が他の人のレビュー待ちになった',
  checks: 'PR が bot・CI 待ちになった',
  changes: 'PR に修正の依頼が来た',
  merge: 'PR が承認された（マージできる）',
  'ci-failed': 'PR の CI が落ちた',
  merged: 'PR がマージされた',
  closed: 'PR がマージされずに閉じられた',
};
/* The most lines the timeline draws; the rest are counted. */
const WORK_AWAY_SHOWN = 20;
/* How far a message may lie behind the turn's end and still be what the agent said at it. */
const WORK_AWAY_MESSAGE_SLACK = 60;

/* A time as seconds, whichever shape it came in: the ledger and the phases keep epoch seconds, gates and the task
   record keep `YYYYMMDDTHHMMSSZ` stamps. Null when it is neither. */
const awaySecs = t => typeof t === 'number' ? (Number.isFinite(t) ? t : null) : stampSecs(t);

/* Seconds as a stamp, for `when` and `ago`, which read stamps. */
const secsStamp = secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+/, '');

/* `-Infinity` while the person has neither left the row nor acted on it: there is nothing to say was missed. */
function workAwaySince(mark, actedAt) {
  const left = Number.isFinite(mark?.left) ? mark.left : -Infinity;
  return Math.max(left, Number.isFinite(actedAt) ? actedAt : -Infinity);
}

/* The first line of what the agent said, cut short. */
function awayFirstLine(message) {
  const line = String(message || '').split('\n').map(x => x.trim()).find(Boolean) || '';
  return line.length > 100 ? `${line.slice(0, 100)}…` : line;
}

/* What happened to the entry after `since`, oldest first: `{ kind, at, … }`. `until`, when given, leaves out what came after it: the
   panel of a row that is open must not list what happens during this visit. */
function workAwayEvents(e, since, until = Infinity) {
  if (!Number.isFinite(since)) return [];
  const out = [];
  const add = (at, ev) => { if (Number.isFinite(at) && at > since && at <= until) out.push({ at, ...ev }); };
  for (const p of e.session?.phases || []) if (Array.isArray(p)) add(awaySecs(p[1]), { kind: 'phase', phase: p[0] });
  // Open gates only: one opened and answered while away is not in the work document, and shows only as the last action.
  for (const g of e.gates) add(awaySecs(g.since), { kind: 'gate', gate: g.gate, title: g.title || '' });
  const a = e.session?.present ? e.session.agentSession : null;
  if (a && !a.error) {
    // `updatedAt` is when the status last changed (docs/session-state.md), so it is when each of these began.
    const at = awaySecs(a.updatedAt);
    if (a.status === 'waiting') add(at, { kind: 'permission', request: a.request || '' });
    else if (a.status === 'failed') add(at, { kind: 'failed' });
    // As `workItems`: a turn that ended at an open gate is the gate's, and a hub has no turn to clear.
    else if (a.status === 'done' && !e.isHub && !e.gates.length) {
      const said = awaySecs(a.lastMessageAt);
      // A message from an earlier turn is not what the agent said now.
      const message = a.lastMessage && (said == null || said >= (at ?? -Infinity) - WORK_AWAY_MESSAGE_SLACK) ? a.lastMessage : '';
      add(at, { kind: 'done', message });
    }
  }
  const t = e.task;
  if (t) {
    // A live card has its turn; a finished one only its state. Without `prTurnAt` the record has not seen it change.
    const turn = t.prTurn || (t.prState === 'merged' || t.prState === 'closed' ? t.prState : null);
    if (turn) add(awaySecs(t.prTurnAt), { kind: 'pr', turn });
  }
  return out.sort((x, y) => x.at - y.at);
}

/* How many things the row's badge counts: 0 for the row the person has open (leaving writes `left`, which clears it),
   for a hub, and for a row that is only a gate. */
function workAwayCount(e, mark, isOpen) {
  if (isOpen || e.isHub || e.ref.startsWith(WORK_GATE_REF)) return 0;
  return workAwayEvents(e, workAwaySince(mark, workActedAt(e))).length;
}

/* An event in words. */
function workAwayText(ev) {
  switch (ev.kind) {
    case 'phase': return `工程が${PHASE_LABEL[ev.phase] || ev.phase}に入った`;
    case 'gate': {
      const title = ev.title ? `『${ev.title}』` : '';
      return ev.gate === 'question' ? `質問が来た${title}` : `${kindOf(ev.gate)[0]}が開いた${title}`;
    }
    case 'permission': return ev.request ? `許可を求めた — ${ev.request}` : '許可を求めた';
    case 'done': {
      const line = awayFirstLine(ev.message);
      return line ? `worker が手を止めた — 『${line}』` : 'worker が手を止めた';
    }
    case 'failed': return 'worker がエラーで止まった';
    case 'pr': return WORK_PR_MOVED[ev.turn] || 'PR が動いた';
    default: return '';
  }
}

/* The last thing the person did on the entry, even when it is older than `left`, as context: `{ at, text }` or null.
   `answered` are the task's gates as the panel loaded them (answered ones carry `answeredAt`). */
function workLastAction(e, answered = []) {
  let best = null;
  const take = (at, text) => { if (Number.isFinite(at) && (!best || at > best.at)) best = { at, text }; };
  for (const g of answered) {
    const label = kindOf(g.kind)[0];
    const title = g.title ? `『${g.title}』` : '';
    if (g.answeredAt && g.decision) {
      const said = DECISION[g.decision] || g.decision;
      // 「質問『t』に答えた」 reads; 「質問『t』を答えた」 does not.
      const particle = ['answer', 'terminal', 'ask'].includes(g.decision) ? 'に' : g.decision === 'choice' ? 'で' : 'を';
      take(awaySecs(g.answeredAt), `${label}${title}${particle}${said}`);
    }
    for (const a of g.answers || []) take(awaySecs(a.answeredAt), `${label}の記録${title}を差し戻した`);
  }
  // What the page has not loaded the answered gates for still says when the last one was answered.
  take(awaySecs(e.task?.gateAnsweredAt), '判定を返した');
  take(e.session?.present ? e.session.agentSession?.lastPromptAt : null, 'ターミナルで指示した');
  const park = parkOf(e.task);
  if (park) take(awaySecs(park.since), `置いた — ${parkText(park)}`);
  return best;
}

/* 「worker が実装中」, and 「worker が PR 中」 with the space the other PR phrases have. */
function phaseWord(phase) {
  const label = PHASE_LABEL[phase] || phase;
  return `${/^[A-Za-z0-9]/.test(label) ? ' ' : ''}${label}${/[A-Za-z0-9]$/.test(label) ? ' ' : ''}中`;
}

/* The statuses of the agent ledger this page knows: the keys of AGENT_STATES in sessions.js (a test keeps them equal). */
const WORK_NOW_STATUSES = ['running', 'waiting', 'idle', 'done', 'failed'];

/* What the entry waits on now, first match first: `{ kind, text }`. `now` is the server's seconds. */
function workNowOf(e, now) {
  const t = e.task;
  const s = e.session;
  const a = s?.present ? s.agentSession : null;
  const live = a && !a.error ? a : null;
  // Running with nothing usable from its hooks (no row, unreadable, a word this page does not know): not said to be at work.
  const unknown = !!s?.present && !(live && WORK_NOW_STATUSES.includes(live.status));
  const from = secs => Number.isFinite(secs) && Number.isFinite(now) ? `（${agoLabel(minutesSince(secs, now))}から）` : '';
  const park = parkOf(t);
  if (park) return { kind: 'parked', text: `置いている — ${parkText(park)}${from(awaySecs(park.since))}` };
  const gate = e.gates[0];
  if (gate) return { kind: 'gate', text: gate.gate === 'question' ? '質問があなたの答え待ち' : `${kindOf(gate.gate)[0]}があなたの判定待ち` };
  if (live?.status === 'waiting') return { kind: 'permission', text: `worker が許可を待っている${live.request ? ` — ${live.request}` : ''}` };
  if (live?.status === 'failed') return { kind: 'failed', text: 'worker がエラーで止まっている' };
  if (t?.waitsOnPerson && WORK_PR_NOW[t.prTurn]) return { kind: 'pr', text: WORK_PR_NOW[t.prTurn] };
  if (live?.status === 'done') return { kind: 'done', text: 'worker は次の指示待ち' };
  if (live && s.phase && !unknown) return { kind: 'running', text: `worker が${phaseWord(s.phase)}${from(awaySecs(s.phaseAt))}` };
  if (t?.prTurn === 'checks') return { kind: 'checks', text: 'PR は bot・CI 待ち' };
  if (t?.prTurn === 'other-reviewer') return { kind: 'checks', text: 'PR は他の人のレビュー待ち' };
  if (!s?.present) return { kind: 'none', text: 'worker はいません' };
  if (unknown) return { kind: 'unknown', text: 'worker の状態は不明' };
  return { kind: 'running', text: 'worker が動いている' };
}
function workNowText(e, now) {
  return workNowOf(e, now).text;
}

/* Everything the panel's block draws for a task, or null when there is nothing the person could have missed (never left
   the row, never acted on it). `left` is when they left it, null when only acted. */
function workAwayModel(e, mark, answered, now, until = Infinity) {
  const since = workAwaySince(mark, workActedAt(e));
  if (!Number.isFinite(since)) return null;
  const events = workAwayEvents(e, since, until);
  const last = workLastAction(e, answered);
  return {
    // Said only when leaving is the boundary: after a later act, "left N ago" would be a time the timeline does not start at.
    left: Number.isFinite(mark?.left) && mark.left >= (last?.at ?? -Infinity) ? mark.left : null,
    last,
    events: events.slice(0, WORK_AWAY_SHOWN),
    more: Math.max(0, events.length - WORK_AWAY_SHOWN),
    now: workNowText(e, now),
  };
}

/* How a parent's children are counted by what each waits on, in the order they are said. */
const WORK_NOW_KINDS = [
  ['gate', '判定待ち'], ['permission', '許可待ち'], ['failed', 'エラー'], ['pr', 'PR の対応待ち'], ['done', '指示待ち'],
  ['running', '作業中'], ['unknown', '状態不明'], ['checks', 'PR は bot・他のレビュー待ち'], ['parked', '置いている'], ['none', 'worker なし'],
];

/* The block of a parent: the children's events, each child by its own `since`, time-sorted with the child's title; what
   each waits on, counted; the newest act among them. A kid is `{ title, entry, mark, mergedAt }`: `entry` is null for a
   child that has no row (a merged one), which then has only its PR's merge, and only if the person left that row once
   (its mark). Null when no child has been left or acted on. */
function workParentAway(kids, now) {
  let any = false;
  let last = null;
  const events = [];
  const counts = new Map();
  for (const k of kids) {
    if (k.entry) {
      const kind = workNowOf(k.entry, now).kind;
      counts.set(kind, (counts.get(kind) || 0) + 1);
      const act = workLastAction(k.entry, []);
      if (act && (!last || act.at > last.at)) last = { ...act, title: k.title };
    }
    const since = workAwaySince(k.mark, k.entry ? workActedAt(k.entry) : -Infinity);
    if (!Number.isFinite(since)) continue;
    any = true;
    const mine = k.entry ? workAwayEvents(k.entry, since)
      : Number.isFinite(k.mergedAt) && k.mergedAt > since ? [{ kind: 'pr', turn: 'merged', at: k.mergedAt }] : [];
    for (const ev of mine) events.push({ ...ev, title: k.title });
  }
  if (!any) return null;
  events.sort((x, y) => x.at - y.at);
  const waits = WORK_NOW_KINDS.filter(([kind]) => counts.has(kind)).map(([kind, label]) => `${label} ${counts.get(kind)}`).join(' · ');
  // Only merged children with no row, and nothing since: there is nothing to draw.
  if (!events.length && !last && !waits) return null;
  return { last, events: events.slice(0, WORK_AWAY_SHOWN), more: Math.max(0, events.length - WORK_AWAY_SHOWN), now: waits };
}
