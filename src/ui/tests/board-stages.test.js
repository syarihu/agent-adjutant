/* The stages of the board and the order of their cards (src/ui/core.js `agentColOf`, `waitingSinceMs`; board-order.js; the
   stack chip, popover and list head of cards.js, stack-pop.js, board-list.js), run with `node --test`. The page's scripts share one
   global scope, so they are loaded into one context the way the browser does; core.js has the page's side effects, so only the
   block of the stages is cut out of it. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');
const cut = (src, re) => {
  const m = src.match(re);
  assert.ok(m, `not found: ${re}`);
  return m[0];
};

const listeners = [];
const els = {};
const ctx = vm.createContext({
  TextEncoder, TextDecoder, URL, window: { addEventListener() {} }, state: {}, prefs: { boardView: 'list', boardGroup: false },
  document: { addEventListener: (t, fn) => listeners.push([t, fn]), getElementById: id => els[id] || null, querySelectorAll: () => [] },
});
const run = (code, name) => vm.runInContext(code, ctx, { filename: name });
run(read('util.js'), 'util.js');
const core = read('core.js');
run(cut(core, /^const BOARD_COL_W = [^\n]*\nconst clampBoardColWidth = [^\n]*\n/m), 'core.js');
run(cut(core, /^\/\* The stages of the board, in the order[\s\S]*?(?=^function columnOf)/m), 'core.js');
const task = read('task-view.js');
run([
  cut(task, /^function issueNumberOf[\s\S]*?\n\}\n/m),
  cut(task, /^function prNumberOf[\s\S]*?\n\}\n/m),
  cut(task, /^function prStateOf[\s\S]*?\n\}\n/m),
  cut(task, /^const prRefNumber = [^\n]*\n/m),
  cut(task, /^function issueLabelOf[\s\S]*?\n\}\n/m),
].join('\n'), 'task-view.js');
const cards = read('cards.js');
run([
  cut(cards, /^function waitBadgeHtml[\s\S]*?\n\}\n/m),
  cut(cards, /^const slugKeyOf = [^\n]*\n/m),
  cut(cards, /^function stackChipHtml[\s\S]*?\n\}\n/m),
].join('\n'), 'cards.js');
run(read('board-order.js'), 'board-order.js');
run(read('stack-pop.js'), 'stack-pop.js');
run('const pageHub = () => null; let selectedTaskId = null;', 'page.js');
run(cut(read('task-panel.js'), /^function markSelectedCards\(\) \{[\s\S]*?\n\}\n/m), 'task-panel.js');
// board-list.js is loaded whole: its helpers are called with what each needs in the page, stood in for here.
run('const stuckOf = () => null; const sessionOfTask = () => null; const sessionState = () => null; const savePrefs = () => {}; const renderBoard = () => {}; const render = () => {}; let showOlderDone = false;', 'stubs.js');
run(read('board-list.js'), 'board-list.js');
const { clampBoardColWidth, onStackFocusOut, onStackScroll, markSelectedCards, agentColOf, waitingSinceMs, orderStage, stackOf, parentGroupsOf, stackChipHtml, waitBadgeHtml, stackPopParts, AGENT_COLUMNS, AGENT_COL_OF_PHASE, syncBoardHead } = vm.runInContext(
  '({ clampBoardColWidth, onStackFocusOut, onStackScroll, markSelectedCards, agentColOf, waitingSinceMs, orderStage, stackOf, parentGroupsOf, stackChipHtml, waitBadgeHtml, stackPopParts, AGENT_COLUMNS, AGENT_COL_OF_PHASE, syncBoardHead })', ctx);

const stamp = secs => new Date(secs * 1000).toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
const T0 = Date.UTC(2026, 9, 1) / 1000;
const t = (id, extra = {}) => ({ id, title: `Task ${id}`, status: 'dispatched', worktree: `/w/${id}`, updatedAt: stamp(T0), ...extra });
const gate = (taskId, kind, humanCol, extra = {}) => ({ id: `g-${taskId}-${kind}`, task: taskId, kind, humanCol, openedAt: stamp(T0), ...extra });
const worker = (taskId, phase, extra = {}) => ({ worktree: `/w/${taskId}`, task: taskId, phase, present: true, ...extra });
const at = (task, data = {}) => agentColOf(task, { gates: [], workers: [], ...data });

test('the six stages are in the order of the person first, and every phase has a stage', () => {
  assert.deepEqual(AGENT_COLUMNS.map(c => c.id), ['me', 'rev', 'ai', 'work', 'before', 'done']);
  assert.deepEqual(AGENT_COLUMNS.map(c => c.label), ['セルフレビュー', 'コードレビュー待ち', 'AIレビュー', '計画・実装', '着手前', '完了']);
  for (const phase of ['plan', 'implement', 'self-review', 'verify', 'pr', 'pr-bots', 'review', 'report']) {
    assert.ok(AGENT_COLUMNS.some(c => c.id === AGENT_COL_OF_PHASE[phase]), phase);
  }
});

test('status decides 着手前 and 完了 before anything else', () => {
  assert.equal(at(t('1', { status: 'backlog' })), 'before');
  assert.equal(at(t('1', { status: 'queued', waitsOnPerson: true })), 'before');
  assert.equal(at(t('1', { status: 'done', pr: 'u' })), 'done');
  assert.equal(at(t('1', { status: 'cancelled' })), 'done');
  assert.equal(at(null), 'before');
});

test('a diff or verify gate that waits, or a PR on the person turn, is セルフレビュー whatever the phase says', () => {
  const w = [worker('1', 'implement')];
  assert.equal(at(t('1'), { gates: [gate('1', 'diff', 'diff')], workers: w }), 'me');
  assert.equal(at(t('1'), { gates: [gate('1', 'verify', 'verify')], workers: w }), 'me');
  // The worker is in pr-bots, and the PR is approved and ready to merge: the person's.
  const merge = t('1', { pr: 'https://x/pull/1', waitsOnPerson: true, prTurn: 'merge', prStatus: { state: 'open', review: 'approved' } });
  assert.equal(at(merge, { workers: [worker('1', 'pr-bots')] }), 'me');
  assert.equal(at(t('1', { waitsOnPerson: true, prTurn: 'changes', pr: 'u' }), { workers: [worker('1', 'review')] }), 'me');
});

test('a plan gate, or the plan and implement phases, are 計画・実装; self-review and verify are AIレビュー', () => {
  assert.equal(at(t('1'), { gates: [gate('1', 'plan', 'plan')] }), 'work');
  assert.equal(at(t('1'), { gates: [gate('1', 'question', 'question')], workers: [worker('1', 'implement')] }), 'work');
  assert.equal(at(t('1'), { workers: [worker('1', 'plan')] }), 'work');
  assert.equal(at(t('1'), { workers: [worker('1', 'implement')] }), 'work');
  assert.equal(at(t('1'), { workers: [worker('1', 'self-review')] }), 'ai');
  assert.equal(at(t('1'), { workers: [worker('1', 'verify')] }), 'ai');
});

test('with a PR the review decides: asked for or given is コードレビュー待ち, bots and CI are AIレビュー, merged is 完了', () => {
  const pr = (prStatus, extra = {}) => t('1', { pr: 'https://x/pull/1', prStatus: { state: 'open', ...prStatus }, ...extra });
  const w = { workers: [worker('1', 'pr-bots')] };
  assert.equal(at(pr({ review: 'required' }), w), 'rev');
  assert.equal(at(pr({ review: 'approved' }), w), 'rev');
  assert.equal(at(pr({ review: 'none' }), w), 'ai');
  assert.equal(at(pr({ review: 'none' }, { prTurn: 'draft' }), w), 'ai');
  assert.equal(at(pr({ state: 'merged', review: 'approved' }), w), 'done');
  assert.equal(at(pr({}), { workers: [worker('1', 'review')] }), 'rev');
  // No PR state read yet: the bots' and CI's.
  assert.equal(at(t('1', { pr: 'https://x/pull/1' }), w), 'ai');
});

test('CI pending does not move a card: the stage follows the review, not prTurn', () => {
  const w = { workers: [worker('1', 'pr-bots')] };
  const pending = prTurn => t('1', { pr: 'https://x/pull/1', prTurn, prStatus: { state: 'open', review: 'required', ci: { pass: 1, fail: 0, pending: 2 } } });
  assert.equal(at(pending('checks'), w), 'rev');
  assert.equal(at(pending('other-reviewer'), w), 'rev');
});

test('a pr or report phase with no PR yet is AIレビュー; nothing to go by is 計画・実装', () => {
  assert.equal(at(t('1'), { workers: [worker('1', 'pr')] }), 'ai');
  assert.equal(at(t('1'), { workers: [worker('1', 'report')] }), 'ai');
  assert.equal(at(t('1'), {}), 'work');
  assert.equal(at(t('1', { status: 'pr' }), {}), 'ai');
});

test('a Jules task with a PR is read like any other, and before it by its session', () => {
  const jules = (extra = {}) => t('1', { executor: 'jules', ...extra });
  assert.equal(at(jules({ pr: 'https://x/pull/1', prStatus: { state: 'open', review: 'required' } })), 'rev');
  assert.equal(at(jules({ pr: 'https://x/pull/1', waitsOnPerson: true })), 'me');
  assert.equal(at(jules({ pr: 'https://x/pull/1', prStatus: { state: 'open', review: 'none' } })), 'ai');
  assert.equal(at(jules({ jules: { state: 'PLANNING' } })), 'work');
  assert.equal(at(jules({ jules: { state: 'IN_PROGRESS' } })), 'work');
  assert.equal(at(jules({ jules: { state: 'COMPLETED' } })), 'ai');
  assert.equal(at(jules({}), { gates: [gate('1', 'diff', 'diff')] }), 'me');
});

test('a session with no task is never in セルフレビュー', () => {
  const bareCol = vm.runInContext(`(${read('board.js').match(/^const bareCol = [\s\S]*?\n\};/m)[0].replace(/^const bareCol = /, '').replace(/;$/, '')})`, ctx);
  for (const phase of ['plan', 'implement', 'self-review', 'verify', 'pr', 'pr-bots', 'report', 'review', undefined, 'unknown']) {
    assert.notEqual(bareCol({ phase }), 'me', String(phase));
  }
  assert.equal(bareCol({ phase: 'plan' }), 'work');
  assert.equal(bareCol({ phase: 'verify' }), 'ai');
  assert.equal(bareCol({ phase: 'review' }), 'rev');
  assert.equal(bareCol({ phase: undefined }), 'work');
});

test('the wait starts at the gate, else the PR turn, else the phase, else the last write', () => {
  const data = { gates: [gate('1', 'diff', 'diff', { openedAt: stamp(T0 + 100) })], workers: [worker('1', 'verify', { phaseAt: T0 + 50 })] };
  assert.equal(waitingSinceMs(t('1', { prTurnAt: stamp(T0 + 70), waitsOnPerson: true }), data), (T0 + 100) * 1000);
  const pr = { workers: data.workers, gates: [] };
  assert.equal(waitingSinceMs(t('1', { prTurnAt: stamp(T0 + 70), waitsOnPerson: true }), pr), (T0 + 70) * 1000);
  // Not on the person's turn: the PR's change says nothing about a wait.
  assert.equal(waitingSinceMs(t('1', { prTurnAt: stamp(T0 + 70) }), pr), (T0 + 50) * 1000);
  assert.equal(waitingSinceMs(t('1', { updatedAt: stamp(T0 + 5) }), { workers: [], gates: [] }), (T0 + 5) * 1000);
});

const waiting = (id, mins, extra = {}) => t(id, { waitsOnPerson: true, prTurn: 'merge', prTurnAt: stamp(T0 + mins * 60), ...extra });
const agent = (id, extra = {}) => t(id, extra);
const ids = list => list.map(x => x.id);

test('what waits on the person comes first, the oldest wait at the top; then the agent side, the newest phase first', () => {
  const data = { gates: [], workers: [worker('a1', 'verify', { phaseAt: T0 + 10 }), worker('a2', 'verify', { phaseAt: T0 + 90 })], parents: [] };
  const list = [agent('a1'), waiting('w-new', 300), agent('a2'), waiting('w-old', 20), waiting('w-mid', 100)];
  assert.deepEqual(ids(orderStage(list, data)), ['w-old', 'w-mid', 'w-new', 'a2', 'a1']);
});

const group = (children, extra = {}) => ({ key: 'o/r#1', url: 'u', number: 1, title: 'Parent', stacked: true, merged: 0, total: children.length, hub: 'h', children, ...extra });
const kid = (id, extra = {}) => ({ hub: 'h', id, title: `Task ${id}`, merged: false, progress: 'pr', ...extra });
const child = (id, extra = {}) => ({ parentIssue: { key: 'o/r#1', hub: 'h', number: 1, title: 'Parent' }, ...extra, id, title: `Task ${id}` });

test('a stack stays together, root first, and is placed by its longest wait', () => {
  const data = { gates: [], workers: [], parents: [group([kid('a', { base: 'main' }), kid('b', { on: 'a', onHub: 'h' }), kid('c', { on: 'b', onHub: 'h' })])] };
  // Listed with the stack scattered and a waiting task between its members. The stack's longest wait is c's (since 10), older
  // than x's (since 60) and younger than y's (since 5): the block sits between them, in the stack's order, not the list's.
  const list = [waiting('x', 60), child('c', waiting('c', 10)), child('a', agent('a')), waiting('y', 5), child('b', waiting('b', 300))];
  assert.deepEqual(ids(orderStage(list, data)), ['y', 'a', 'b', 'c', 'x']);
  // Only the members that are in the stage are kept together.
  assert.deepEqual(ids(orderStage([child('c', waiting('c', 10)), waiting('x', 60), child('a', agent('a'))], data)), ['a', 'c', 'x']);
});

test('a stack is read from the parent group the board lists, by hub and id and by board', () => {
  const g = group([kid('a', { base: 'main' }), kid('b', { on: 'a', onHub: 'h' }), kid('z')]);
  const data = { parents: [g] };
  const s = stackOf(child('b'), data);
  assert.deepEqual([s.at, s.chain.map(c => c.id)], [1, ['a', 'b']]);
  assert.equal(s.root.base, 'main');
  // A child that is cut from nothing and has nothing cut from it is in no stack; nor is a task of another hub.
  assert.equal(stackOf(child('z'), data), null);
  assert.equal(stackOf({ ...child('b'), parentIssue: { key: 'o/r#1', hub: 'other' } }, data), null);
  // 「すべて」: the same key on another board is another group.
  assert.equal(stackOf({ ...child('b'), _slug: 's2' }, { parents: [{ ...g, _slug: 's1' }] }), null);
  assert.equal(stackOf({ ...child('b'), _slug: 's1' }, { parents: [{ ...g, _slug: 's1' }] }).at, 1);
  assert.equal(stackOf(child('b'), { parents: [{ ...g, stacked: false }] }), null);
});

test('the chip says the place in the stack, and is a button with a name', () => {
  const data = { parents: [group([kid('a'), kid('b', { on: 'a', onHub: 'h' }), kid('c', { on: 'b', onHub: 'h' })])] };
  ctx.state = data;
  const html = stackChipHtml({ ...child('b'), _slug: undefined });
  assert.match(html, /^<button type="button" class="stack-chip" data-stack="b"/);
  assert.match(html, /aria-label="スタック 2\/3 の依存を見る"/);
  assert.match(html, /stacks<\/span>2\/3<\/button>/);
  assert.equal(stackChipHtml(child('zzz')), '');
  ctx.state = {};
});

test('the wait badge says minutes, hours or days, and warns after an hour and a day', () => {
  const badge = mins => {
    ctx.state = { gates: [], workers: [] };
    const tk = waiting('1', 0, { prTurnAt: stamp(Date.now() / 1000 - mins * 60) });
    return waitBadgeHtml(tk);
  };
  assert.match(badge(5), /^<span class="wait-badge" .*>5分待ち<\/span>$/);
  assert.match(badge(90), /class="wait-badge mid".*1時間待ち/);
  assert.match(badge(3 * 1440), /class="wait-badge long".*3日待ち/);
  ctx.state = {};
});

test('the popover lists the stack top first, marks the task, and names the base; a step with no task is no button', () => {
  const g = group([kid('a', { base: 'main', branch: 'br-a' }), kid('b', { on: 'a', onHub: 'h', branch: 'br-b' }), kid('m', { on: 'b', onHub: 'h', merged: true, branch: 'br-m' })]);
  ctx.state = { parents: [g], tasks: [
    child('a', { pr: 'https://github.com/o/r/pull/11', prStatus: { state: 'open' }, issueUrl: 'https://github.com/o/r/issues/5' }),
    child('b', { pr: 'https://github.com/o/r/pull/12', prStatus: { state: 'draft' }, status: 'dispatched' }),
  ], workers: [], gates: [] };
  const parts = stackPopParts(ctx.state.tasks[0]);
  assert.equal(parts.label, 'スタック・親 #1');
  assert.match(parts.html, /Stack · 親 #1/);
  const steps = parts.html.split('<li ').slice(1);
  assert.equal(steps.length, 3);
  // Top of the stack first: m (no task on this board), then b, then a, which is the task and is marked.
  assert.match(steps[0], /<span class="stack-go">/);
  assert.doesNotMatch(steps[0], /data-goto=/);
  assert.match(steps[1], /data-goto="b"/);
  assert.match(steps[1], /#12 · br-b · AIレビュー/);
  assert.match(steps[2], /stack-step here" aria-current="true"/);
  assert.match(steps[2], /#5 Task a/);
  assert.match(steps[2], /#11 · br-a/);
  assert.match(parts.html, /<code>main<\/code>/);
  // No base known, no foot.
  ctx.state.parents = [group([kid('a'), kid('b', { on: 'a', onHub: 'h' })])];
  assert.doesNotMatch(stackPopParts(ctx.state.tasks[0]).html, /stack-foot/);
  ctx.state = {};
});

test('the list groups by parent in the order the tasks have, and keeps a stack in one group', () => {
  const data = { parents: [group([kid('a'), kid('b', { on: 'a', onHub: 'h' })])] };
  const other = { parentIssue: { key: 'o/r#2', number: 2, title: 'Other' } };
  const gs = parentGroupsOf([t('1', other), child('a'), t('2'), child('b'), t('3', other)], data);
  assert.deepEqual(gs.map(g => [g.key, g.stacked, ids(g.tasks)]), [
    ['o/r#2', false, ['1', '3']], ['o/r#1', true, ['a', 'b']], ['', false, ['2']],
  ]);
});

test('the switch marks the one drawing that is shown, and the grouping box is only the list\'s', () => {
  for (const id of ['board-agent', 'board-list', 'board-group-opt', 'board-group']) els[id] = { hidden: false, checked: false };
  const btns = ['list', 'cols'].map(v => ({ dataset: { boardView: v }, attrs: {}, on: false,
    classList: { toggle(_, on) { btns[v === 'list' ? 0 : 1].on = on; } }, setAttribute(k, val) { this.attrs[k] = val; } }));
  ctx.document.querySelectorAll = () => btns;
  ctx.prefs.boardGroup = true;
  syncBoardHead(true);
  assert.deepEqual([els['board-agent'].hidden, els['board-list'].hidden, els['board-group-opt'].hidden, els['board-group'].checked], [true, false, false, true]);
  assert.deepEqual(btns.map(b => b.attrs['aria-pressed']), ['true', 'false']);
  syncBoardHead(false);
  assert.deepEqual([els['board-agent'].hidden, els['board-list'].hidden, els['board-group-opt'].hidden], [false, true, true]);
  assert.deepEqual(btns.map(b => b.attrs['aria-pressed']), ['false', 'true']);
});

test('the list head names the seven columns, and the narrow ones are the two marked for hiding', () => {
  const head = vm.runInContext('LIST_HEAD', ctx);
  assert.deepEqual([...head.matchAll(/<span[^>]*>([^<]+)<\/span>/g)].map(m => m[1]), ['待ち時間', 'スタック', 'キー', 'タイトル', '親', '待っているもの', 'PR・状態']);
  assert.equal([...head.matchAll(/class="c-(parent|for)"/g)].length, 2);
});

test('the selected task is marked on its card and on its row, and unmarked when the panel closes', () => {
  const item = id => ({ dataset: { id }, on: false, classList: { toggle(_, on) { this.owner.on = on; } } });
  const items = [item('a'), item('b'), item('a')];
  items.forEach(i => { i.classList.owner = i; });
  ctx.document.querySelectorAll = sel => (/\.list-row/.test(sel) ? items : items.slice(0, 1));
  vm.runInContext("selectedTaskId = 'a'", ctx);
  markSelectedCards();
  assert.deepEqual(items.map(i => i.on), [true, false, true]);
  vm.runInContext('selectedTaskId = null', ctx);
  markSelectedCards();
  assert.deepEqual(items.map(i => i.on), [false, false, false]);
  ctx.document.querySelectorAll = () => [];
});

test('the popover closes when focus goes elsewhere or the chip is scrolled away, but not when focus goes to the chip or the chip stays', () => {
  const chipRect = { left: 100, top: 50 };
  const chip = { dataset: { stack: 'b', stackSlug: '' }, attrs: {}, setAttribute(k, v) { this.attrs[k] = v; }, getBoundingClientRect: () => chipRect, closest: sel => (sel === '.stack-chip' ? chip : null), focus() {} };
  const inside = { closest: () => null };
  const pop = { hidden: false, contains: x => x === inside };
  els['stack-pop'] = pop;
  ctx.document.querySelectorAll = () => [chip];
  const open = () => { pop.hidden = false; vm.runInContext("stackPop = { id: 'b', slug: '' }; stackPopAt = { left: 100, top: 50 }", ctx); };
  const closed = () => pop.hidden;
  open();
  onStackFocusOut({ relatedTarget: chip });
  assert.equal(closed(), false, 'the chip: the click toggles it');
  onStackFocusOut({ relatedTarget: null });
  assert.equal(closed(), false, 'no target: a press elsewhere closes it');
  onStackFocusOut({ relatedTarget: inside });
  assert.equal(closed(), false);
  onStackFocusOut({ relatedTarget: { closest: () => null } });
  assert.equal(closed(), true, 'something else');
  open();
  onStackScroll({ target: {} });
  assert.equal(closed(), false, 'the chip has not moved');
  chipRect.top = 80;
  onStackScroll({ target: {} });
  assert.equal(closed(), true, 'the chip moved');
  delete els['stack-pop'];
  ctx.document.querySelectorAll = () => [];
});

test('the column width pref is a number in its range, else the default', () => {
  assert.equal(clampBoardColWidth(300), 300);
  assert.equal(clampBoardColWidth(10), 200);
  assert.equal(clampBoardColWidth(99999), 520);
  assert.equal(clampBoardColWidth(260.6), 261);
  for (const bad of [undefined, null, 'wide', NaN, {}, '', 0]) assert.equal(clampBoardColWidth(bad), 280, String(bad));
  assert.equal(clampBoardColWidth('350'), 350);
});

test('a drawing asked for during a column drag is held, and made once when the drag ends', () => {
  const board = read('board.js');
  const pieces = [cut(board, /^function renderBoard\(\) \{[\s\S]*?\n\}\n/m), cut(board, /^let colDrag = false[^\n]*\n/m), cut(board, /^function flushBoardRender\(\) \{[\s\S]*?\n\}\n/m)];
  const local = vm.createContext({ document: { addEventListener() {} }, drawn: 0 });
  vm.runInContext(`function drawBoard() { drawn++; }\n${pieces.join('\n')}`, local);
  const run2 = code => vm.runInContext(code, local);
  run2('renderBoard()');
  assert.equal(run2('drawn'), 1);
  run2('colDrag = true; renderBoard(); renderBoard()');
  assert.equal(run2('drawn'), 1, 'held while dragging');
  run2('flushBoardRender()');
  assert.equal(run2('drawn'), 2, 'one drawing when the drag ends');
  run2('flushBoardRender()');
  assert.equal(run2('drawn'), 2, 'none when nothing was held');
  assert.equal(run2('colDrag'), false);
});
