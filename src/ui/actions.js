// ── the actions, each one logging the adj command it maps to ──────────

async function move(id, to, before) {
  const task = (state.tasks || []).find(t => t.id === id);
  if (!task || !canDrop(columnOf(task), to)) return;
  if (columnOf(task) === 'backlog' && to === 'queued') return openHandoverDialog(id, before);
  if (to === 'backlog') {
    await update(id, { status:'backlog' },
      `adj task update --id ${id} --status backlog`,
      'hub は着手の直前に status を読むため、戻したものはスキップされます');
    return;
  }
  // Reordering inside 待ち: the dropped card takes the position of the one it landed on.
  const order = await queueOrder(id, before);
  if (order == null) return;
  await update(id, { order }, `adj task update --id ${id} --order ${order}`,
    'hub がキューの先頭から取得する順序です');
}

/* The order that puts a task just before `before` in 待ち, or at its end without one.
   Orders are not positions: a new task takes the highest order of every task plus one, so the
   queue can read 12, 15, 20. The task takes the gap below the one it lands on. When there is
   no gap, that one and the ones after it move down first, so no two queued tasks share an
   order. They move from the last one up, each into an order nobody holds, so a request that
   fails part way leaves the queue in the same sequence with no order shared. Null then. */
async function queueOrder(id, before) {
  const queue = (state.tasks || []).filter(t => t.status === 'queued' && t.id !== id)
    .sort((a, b) => (a.order || 0) - (b.order || 0));
  const at = before ? queue.findIndex(t => t.id === before) : -1;
  if (at < 0) return (queue.length ? (queue[queue.length - 1].order || 0) : 0) + 1;
  const floor = at > 0 ? (queue[at - 1].order || 0) + 1 : 0;
  const target = queue[at].order || 0;
  if (target > floor) return target - 1;
  const shifts = [];
  let next = floor + 1;
  for (const t of queue.slice(at)) {
    if ((t.order || 0) >= next) break;
    shifts.push([t.id, next++]);
  }
  for (const [tid, order] of shifts.reverse()) {
    const line = `adj task update --id ${tid} --order ${order}`;
    try {
      await api(`/api/tasks/${encodeURIComponent(tid)}`, {
        method:'POST', body: JSON.stringify({ order }),
      });
      note(line, false, '間に入れる場所を空けるため後ろへずらしました');
    } catch (e) {
      note(`${line} → ${e.message}`, true);
      return null;
    }
  }
  return floor;
}

let handoverTargetTaskId = null;
let handoverTargetBefore = null;

function openHandoverDialog(id, before = null) {
  const task = (state.tasks || []).find(t => t.id === id);
  if (!task) return;
  handoverTargetTaskId = id;
  handoverTargetBefore = before;
  const dialog = document.getElementById('handover-dialog');
  const titleEl = document.getElementById('handover-task-title');
  const textarea = document.getElementById('handover-instruction');
  if (titleEl) titleEl.textContent = task.title;
  if (textarea) {
    textarea.value = task.instruction || '';
    textarea.onkeydown = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.isComposing) {
        e.preventDefault();
        submitHandover();
      }
    };
  }
  if (dialog) {
    dialog.showModal();
    if (textarea) setTimeout(() => textarea.focus(), 50);
  }
}

function closeHandoverDialog() {
  const dialog = document.getElementById('handover-dialog');
  if (dialog && dialog.open) dialog.close();
  handoverTargetTaskId = null;
  handoverTargetBefore = null;
}

async function submitHandover(e) {
  if (e) e.preventDefault();
  if (!handoverTargetTaskId) return;
  const textarea = document.getElementById('handover-instruction');
  const instruction = textarea ? textarea.value.trim() : '';
  const id = handoverTargetTaskId;
  const before = handoverTargetBefore;
  closeHandoverDialog();
  // Dropped onto a card: the place is made in the queue just before it is handed over.
  const order = await queueOrder(id, before);
  if (order == null) return;
  hand(id, instruction, order);
}

async function hand(id, instruction = null, order = null) {
  // Handed over without a place in mind, the task joins the end of the queue. Its own order
  // dates from when it was created, which would put it ahead of tasks handed over before it.
  if (order == null) order = await queueOrder(id, null);
  const body = { status:'queued' };
  // A string came from a field the person saw, so an empty one clears the instruction kept on
  // the task. Null is a hand-over with no field on screen, which leaves it as it was.
  if (instruction != null) body.instruction = instruction.trim();
  if (order != null) {
    body.order = order;
  }
  const cmd = body.instruction ? `adj task update --id ${id} --status queued --instruction -`
    : body.instruction === '' ? `adj task update --id ${id} --status queued --instruction ''`
    : `adj task update --id ${id} --status queued`;
  await update(id, body, cmd,
    'レコードを queued にして、受信箱に kind:request を配置し hubWake を実行します');
}

async function update(id, body, line, why) {
  try {
    const data = await api(`/api/tasks/${encodeURIComponent(id)}`, {
      method:'POST', body: JSON.stringify(body),
    });
    note(line, false, why + handedNote(data.handed));
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

async function nudgeHub() {
  const line = "adj send --kind next --from dashboard --subject 'start the next queued task if a worker slot is free'";
  try {
    const data = await api('/api/hub/next', { method: 'POST' });
    note(line, false, '枠が空いていれば待ちの先頭を着手' + handedNote(data.handed));
    await refresh();
  } catch (e) {
    note(`${line} → ${e.message}`, true);
  }
}

async function refreshPrs(e) {
  const line = 'adj task refresh';
  const button = e?.currentTarget;
  if (button) button.disabled = true;
  try {
    const data = await api('/api/refresh', { method: 'POST' });
    const left = [['open', 'open'], ['closed', '閉じた'], ['unreadable', '読めない'], ['skipped', '途中で変更'], ['failed', '移せなかった']]
      .filter(([k]) => data[k]?.length)
      .map(([k, label]) => `${label} ${data[k].length}`);
    const moved = data.done.length ? `${data.done.length} 件を完了に` : '完了に移すものは無し';
    note(line, false, moved + (left.length ? ` / 触らなかった: ${left.join('・')}` : ''));
    // One line for all of them: the log keeps five, and the summary above must not scroll off.
    if (data.unreadable?.length) {
      note(`${line}: 状態を読めなかった PR`, true, data.unreadable.map(t => `${t.pr} (${t.error})`).join(' / '));
    }
    if (data.failed?.length) {
      note(`${line}: マージ済みだが完了に移せなかったタスク`, true, data.failed.map(t => `${t.id} (${t.error})`).join(' / '));
    }
    await refresh();
  } catch (err) {
    note(`${line} → ${err.message}`, true);
  } finally {
    if (button) button.disabled = false;
  }
}

function handedNote(handed) {
  if (!handed) return '';
  if (!handed.present) return ' / hub は停止中のため、次回起動時に処理されます';
  return handed.woken ? ' / hub を起動/通知しました' : ' / hub は稼働中。次回の受信箱確認時に処理されます';
}

function openForm() { document.getElementById('form').showModal(); syncForm(); }
function syncForm() {
  const kind = document.querySelector('input[name=kind]:checked').value;
  document.getElementById('f-issue').classList.toggle('hidden', kind !== 'start');
  document.getElementById('f-wtname').classList.toggle('hidden', kind === 'start');
}

async function submitForm(e) {
  const f = new FormData(e.target);
  const status = e.submitter?.value || 'backlog';
  const titleText = (f.get('title') || '').trim();
  const bodyText = (f.get('body') || '').trim();
  const body = {
    body: bodyText,
    kind: f.get('kind'),
    doneWhen: f.get('doneWhen'),
    stopAt: f.get('stopAt'),
    autoStart: f.get('autoStart') === 'true',
    status,
  };
  if (titleText) body.title = titleText;
  for (const key of ['issueUrl', 'base', 'parent', 'worktreeName']) {
    const value = (f.get(key) || '').trim();
    if (value) body[key] = value;
  }
  const line = `adj task add` + (titleText ? ` --title '${titleText}'` : '') + ` --kind ${body.kind}` +
    (body.stopAt && body.stopAt !== 'plan' ? ` --stop-at ${body.stopAt}` : '') + (status === 'queued' ? ' --queue' : '');
  try {
    const data = await api('/api/tasks', { method:'POST', body: JSON.stringify(body) });
    note(line, false, (status === 'queued' ? '記録して受信箱へ' : 'Backlog は受信箱へ送信しません') + handedNote(data.handed));
    e.target.reset();
    await refresh();
  } catch (err) {
    note(`${line} → ${err.message}`, true);
  }
}

function note(line, isError, why) {
  log.unshift({ line, isError, why });
  log = log.slice(0, 5);
  document.getElementById('log').innerHTML = log.map(l =>
    `<li class="${l.isError ? 'err' : ''}"><b>${l.isError ? '!' : '$'}</b> ${esc(l.line)}` +
    (l.why ? `   <span style="color:var(--muted)">— ${esc(l.why)}</span>` : '') + '</li>').join('');
  const latestCmd = document.getElementById('sheet-latest-cmd');
  if (latestCmd) {
    latestCmd.textContent = line;
    latestCmd.style.color = isError ? 'var(--md-sys-color-error)' : 'var(--md-sys-color-outline)';
  }
}

