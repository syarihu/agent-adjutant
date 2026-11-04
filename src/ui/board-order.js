/* The order of the cards of a stage, and the stacks the board draws. Pure: everything is read from the arguments (`data` is the
   page's state, as `agentColOf` takes it), so the columns and the list put the same cards in the same order. */

/* The stack a task is in, from the board's `parents`, or null. The server lists a stacked parent's children root first and
   says which sibling each is cut from (`on`, `onHub`); the page does not work stacks out from branches. A child is named by its
   hub and id, since an id is only unique within one hub, and the group by its key and board, as `parentGroupOf` does, so that
   「すべて」 does not mix boards. Returns `{ group, chain, at, root }`: `chain` is the children of the stack root first and `at`
   the task's place in it. */
/* The parent group the board lists for a task's parent (by key, and by board in 「すべて」), or undefined. */
const parentGroupFor = (task, data = state) => {
  const p = task?.parentIssue;
  return p && (data.parents || []).find(g => g.key === p.key && (!task._slug || g._slug === task._slug));
};

function stackOf(task, data = state) {
  const p = task?.parentIssue;
  const group = p && parentGroupFor(task, data);
  if (!group || !group.stacked) return null;
  const kids = group.children || [];
  const same = (c, id, hub) => c.id === id && c.hub === hub;
  const upOf = c => c.on && kids.find(d => same(d, c.on, c.onHub));
  const rootOf = c => { let at = c; for (let i = 0; i <= kids.length && upOf(at); i++) at = upOf(at); return at; };
  const inStack = c => c.on || kids.some(d => same(c, d.on, d.onHub));
  const mine = kids.find(c => same(c, task.id, p.hub));
  if (!mine || !inStack(mine)) return null;
  const root = rootOf(mine);
  const chain = kids.filter(c => inStack(c) && rootOf(c) === root);
  const at = chain.indexOf(mine);
  return at < 0 ? null : { group, chain, at, root };
}

/* The hub a task is a child through: its parent's hub, or the hub that owns a card the repository's board lists for it. */
const childHubOf = t => t.parentIssue?.hub || t.ownerHub?.slug;

/* The task of a child of a stack, among `tasks`, or undefined: a child of another hub's board may not be one of them. */
const taskOfChild = (c, group, tasks) =>
  tasks.find(x => x.id === c.id && childHubOf(x) === c.hub && (!group._slug || x._slug === group._slug));

/* When the worker last entered a phase (ms), else when the task was last written: how new an agent-side card is. */
function phaseSinceMs(t, data = state) {
  const w = workerOf(t, data);
  return w && w.phaseAt ? w.phaseAt * 1000 : updatedMs(t);
}

/* The tasks among `tasks` that `task` comes after without being stacked on them: the siblings its child entry names in `after` (the
   server keeps them to siblings of the same parent and breaks loops), each by its hub (`afterHub`) and id. */
function afterOf(task, tasks, data = state) {
  const group = parentGroupFor(task, data);
  const mine = group && (group.children || []).find(c => c.id === task.id && c.hub === task.parentIssue.hub);
  return (mine?.after || [])
    .map((id, i) => tasks.find(x => x !== task && x.id === id && childHubOf(x) === mine.afterHub?.[i] && x.parentIssue?.key === group.key && x._slug === task._slug))
    .filter(Boolean);
}

/* `tasks` (all in one stage, not 着手前, which keeps the queue's order) in the order they are drawn:
   - what waits on the person first, the oldest wait at the top (`waitingSinceMs`);
   - then the agent side, the newest phase first;
   - tasks of one stacked parent that are in this stage stay together, root first (the order of `parents[].children`), and
     the block is placed by its longest wait;
   - tasks that are not stacked but come after one another (`after`) are a block too, the earlier one first. A task in a stack
     keeps the stack's order, whatever `after` says. Page-side the loops are safe too: each task is placed once. */
function orderStage(tasks, data = state) {
  const placed = new Set();
  const units = [];
  const stacked = new Map(tasks.map(t => [t, stackOf(t, data)]));
  // The `after` edges between tasks that are in no stack, both ways, to find the block a task is in.
  const before = new Map(tasks.map(t => [t, stacked.get(t) ? [] : afterOf(t, tasks, data).filter(x => !stacked.get(x))]));
  const next = new Map(tasks.map(t => [t, []]));
  for (const [t, ps] of before) for (const p of ps) next.get(p).push(t);
  const blockOf = t => {
    const seen = new Set([t]);
    for (const x of seen) for (const y of [...before.get(x), ...next.get(x)]) seen.add(y);
    const out = [];
    // The earlier first: a task goes after its earlier ones, each placed once, so a loop is cut where it closes.
    const visiting = new Set();
    const walk = x => {
      if (out.includes(x) || visiting.has(x)) return;
      visiting.add(x);
      for (const p of before.get(x)) walk(p);
      out.push(x);
    };
    for (const x of tasks) if (seen.has(x)) walk(x);
    return out;
  };
  tasks.forEach((t, i) => {
    if (placed.has(t)) return;
    const st = stacked.get(t);
    let members = st ? st.chain.map(c => taskOfChild(c, st.group, tasks)).filter(Boolean) : blockOf(t);
    if (!members.includes(t)) members = [t];
    members.forEach(x => placed.add(x));
    const waiting = members.filter(x => humanColOf(x, data));
    units.push({
      members, i,
      person: waiting.length > 0,
      since: waiting.length ? Math.min(...waiting.map(x => waitingSinceMs(x, data))) : 0,
      fresh: Math.max(...members.map(x => phaseSinceMs(x, data))),
    });
  });
  units.sort((a, b) => (b.person - a.person) || (a.person ? a.since - b.since : b.fresh - a.fresh) || a.i - b.i);
  return units.flatMap(u => u.members);
}

/* The cards of 着手前 in the order the hub takes them: the queue by its order, then the backlog. */
function orderBefore(tasks) {
  const queued = tasks.filter(t => t.status === 'queued').sort((a, b) => (a.order || 0) - (b.order || 0));
  return [...queued, ...tasks.filter(t => t.status === 'backlog')];
}

/* The parent groups of an ordered run of tasks, for the list's 「親Issueでまとめる」: the order the tasks already have is kept
   (a group sits where its first task is, and a stack, being one parent's, is never split). Tasks with no parent are one group
   with no key. */
function parentGroupsOf(tasks, data = state) {
  const groups = new Map();
  for (const t of tasks) {
    const p = t.parentIssue;
    const id = p ? `${t._slug || ''}|${p.key}` : '';
    if (!groups.has(id)) {
      const group = p ? (data.parents || []).find(g => g.key === p.key && (!t._slug || g._slug === t._slug)) : null;
      groups.set(id, { key: p ? p.key : '', number: p?.number ?? group?.number, title: p?.title ?? group?.title, stacked: !!group?.stacked, tasks: [] });
    }
    groups.get(id).tasks.push(t);
  }
  return [...groups.values()];
}
