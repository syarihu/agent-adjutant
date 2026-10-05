/* ── Actions on a session ── */
const sessBusy = new Set(); // in-flight requests: a second press is ignored and buttons dim
const enc = encodeURIComponent;

/* One row in the flow of the page, above the list: never laid over the panel's terminal. */
function showSessNotice(text, isError = false) {
  const el = sessEl('sess-notice');
  el.hidden = !text;
  el.className = 'sess-notice' + (isError ? ' err' : '');
  el.setAttribute('role', isError ? 'alert' : 'status');
  el.innerHTML = text
    ? `<span class="sess-notice-text">${esc(text)}</span><button type="button" class="btn-m3-text" data-sess-dismiss>閉じる</button>` : '';
}

async function sessAct(key, label, fn) {
  if (sessBusy.has(key)) return;
  sessBusy.add(key);
  renderTaskPanel();
  try {
    await fn();
  } catch (e) {
    note(`${label} → ${e.message}`, true);
    showSessNotice(`${label}: ${e.message}`, true);
  } finally {
    sessBusy.delete(key);
    renderTaskPanel();
  }
}

/* Why the board cannot start `h` (and so cannot reset it), or '' when it can. */
const hubStartWhy = h => h.parent && !h.key ? 'キーが分からないため起動できません。adj hub --hub <キー> で起動してください'
  : !state.hubStart?.available ? 'ボードからの起動は terminal.preset が "tmux" のときだけ使えます' : '';

/* What the board can do about a hub, by the same rule as its row in the hub list: a parent-task
   hub that no checkout reports to any more is closed, whether or not it runs. */
function hubActionOf(s) {
  const h = (state.hubs || []).find(x => x.id === s.id);
  if (!h) return null;
  const present = !!(s.present ?? h.state?.present);
  // A reset started from the rail leaves these two doing nothing until it is over.
  const resetting = !!hubTurnover(h);
  const starting = hubStartingNow(h);
  if (h.parent && !h.children) return { act: 'hub-close', icon: 'close', label: 'hub を閉じる', disabled: starting, title: hubTurnover(h) || (starting ? 'hub を起動しています' : 'この hub を止めて一覧から外します。タスク・gate・受信箱の記録は残ります') };
  if (present) return { act: 'hub-stop', icon: 'stop', label: 'hub を止める', disabled: resetting, title: hubTurnover(h) || 'hub が動いている tmux のペインを閉じます' };
  const why = hubStartWhy(h) || hubWaitWhy(h);
  return { act: 'hub-start', icon: 'play_arrow', label: 'hub を起動', title: why || 'tmux の新しいウィンドウで adj hub を実行します', disabled: !!why };
}

const canResume = s => s.kind === 'worker' && !s.present && !!s.conversation && !!state.sessionResume?.available && !restartingNow(s);

/* A restart on this page is under way for `s`: from the request until the new process shows. */
const restartKey = s => `${boardOfSession(s).slug}/${s.id}`;
function restartingNow(s) {
  if (s.kind === 'hub') {
    const h = (state.hubs || []).find(x => x.id === s.id);
    return !!h && hubRestartPending(h);
  }
  return restartPending(sessRestarting.get(restartKey(s)), s);
}

/* Why `s` cannot be restarted from the board, or '' when it can: the first of these that
   holds, as 再開's. Only a running hub or worker is offered it, so the page asks `canRestart`
   first. */
function restartWhy(s) {
  if (restartingNow(s)) return '再起動しています';
  if (!s.conversation) return '保存された会話がないため再起動できません';
  if (s.kind === 'hub') {
    const h = (state.hubs || []).find(x => x.id === s.id);
    if (h && hubResetting.has(h.id)) return 'hub をリセットしています';
    if (!state.hubResume?.available) return state.hubResume?.reason || 'ボードからは再起動できません';
    return h ? hubStartWhy(h) : '';
  }
  if (!state.sessionResume?.available) return state.sessionResume?.reason || 'ボードからは再起動できません';
  return '';
}
const canRestart = s => (s.kind === 'hub' || (s.kind === 'worker' && !!s.worktree)) && !!(s.present || restartingNow(s));
const linkedWorktree = s => s.kind === 'worker' && !!s.worktree && s.worktree !== state.main;

/* The hub reset as a button entry, shared by the actions menu and the bar above a hub's terminal so
   both carry the same label, tooltip and disabled reason. */
function hubResetButton(s) {
  const h = s.kind === 'hub' && (state.hubs || []).find(x => x.id === s.id);
  if (!h) return null;
  const why = hubStartWhy(h) || hubWaitWhy(h);
  return { act: 'hub-reset', icon: 'fiber_new', label: 'hub をリセット…', title: why || 'hub をリセット：新しい会話で hub を起動し直します（adj hub --new）', disabled: !!why };
}

function sessionButtons(s) {
  const bar = [];
  const menu = [];
  if (canResume(s)) bar.push({ act: 'resume', icon: 'restart_alt', label: '再開', title: '保存された会話を tmux の新しいウィンドウで再開します' });
  if (canRestart(s)) {
    const why = restartWhy(s);
    bar.push({ act: 'restart', icon: 'autorenew', label: 'セッションを再起動…', title: why || '今の会話のまま、止めて起動し直します。新しい Claude Code に切り替えたいときなどに使います', disabled: !!why });
  }
  if (s.kind === 'worker' && s.present && linkedWorktree(s)) bar.push({ act: 'close', icon: 'tab_close', label: 'セッションを閉じる', title: restartingNow(s) ? '再起動しています' : 'worker のタブを閉じます（worktree は残ります）', disabled: restartingNow(s) });
  if (s.kind === 'hub') {
    const hub = hubActionOf(s);
    if (hub) bar.push(hub);
    const reset = hubResetButton(s);
    if (reset) menu.push(reset);
  }
  if (boardTerminalReady(s)) {
    const open = state.sessionOpen;
    bar.push(open?.available
      ? { act: 'open', icon: 'open_in_new', label: '端末で開く', title: `${open.terminal} で開く` }
      : { act: 'open', icon: 'open_in_new', label: '端末で開く', title: 'この端末からは開けません（terminal.attach が未設定で、iTerm2 も見つかりません）', disabled: true });
  }
  if (linkedWorktree(s)) menu.push({ act: 'ide', icon: 'code', label: 'IDE で開く' });
  if (s.worktree) menu.push({ act: 'copy', icon: 'content_copy', label: 'パスをコピー' });
  if (linkedWorktree(s)) menu.push({ act: 'cleanup', icon: 'delete_sweep', label: '片付ける…', title: restartingNow(s) ? '再起動しています' : '', disabled: restartingNow(s) });
  return { bar, menu };
}

const actionButtonHtml = (b, { menu = false, busy = false } = {}) =>
  `<button type="button"${menu ? ' role="menuitem"' : ` class="btn-m3-tonal sess-act"`} data-sess-act="${b.act}"${b.title ? ` title="${esc(b.title)}"` : ''}${b.disabled || busy ? ' disabled' : ''}>` +
  `<span class="material-symbols-outlined" aria-hidden="true">${b.icon}</span><span>${esc(b.label)}</span></button>`;

function runSessionAction(act, s) {
  const key = sessionKey(s);
  const id = enc(s.id);
  switch (act) {
    case 'open':
      return sessAct('open', `端末で開く (${key})`, async () => {
        const data = await api(`/api/sessions/${id}/open`, { method: 'POST', body: '{}' });
        note(`端末で開く (${key})`, false, data.description);
        showSessNotice(data.description || '端末で開きました');
      });
    case 'close':
      if (restartingNow(s)) return note('再起動しています。終わってから操作してください', true);
      return worktreeAct('close', s.worktree);
    case 'resume':
      return sessAct('resume', `セッションを再開 (${key})`, async () => {
        const data = await api(`/api/sessions/${id}/resume`, { method: 'POST', body: '{}' });
        note(`セッションを再開 (${key})`, false, data.description);
        showSessNotice('再開しました' + (data.hubRunning === false ? '。hub は止まっています' : ''));
        await refresh();
      });
    case 'hub-start':
      if ((state.hubs || []).some(x => x.id === s.id && hubStartingNow(x))) return showSessNotice('hub を起動しています');
      return sessAct('hub-start', `hub を起動 (${key})`, async () => {
        const data = await api(`/api/hubs/${id}/start`, { method: 'POST', body: '{}' });
        const text = data.alreadyRunning ? 'hub はすでに動いています' : 'hub を tmux で起動しました';
        note(`hub を起動 (${key})`, false, text);
        showSessNotice(text);
        await refresh();
      });
    case 'restart':
      return s.kind === 'hub' ? openHubStopDialog(s.id, 'restart') : openRestartDialog(s);
    case 'hub-reset':
      return openHubStopDialog(s.id, 'reset');
    case 'hub-stop':
    case 'hub-close':
      return openHubStopDialog(s.id, act === 'hub-stop' ? 'stop' : 'close');
    case 'ide':
      return worktreeAct('ide', s.worktree);
    case 'copy':
      // Without a secure context there is no clipboard object, and the call throws.
      return Promise.resolve().then(() => navigator.clipboard.writeText(s.worktree))
        .then(() => showSessNotice('パスをコピーしました'))
        .catch(e => {
          note(`パスをコピー (${sessionKey(s)}) → ${e.message}`, true);
          showSessNotice(`パスをコピーできませんでした: ${e.message}`, true);
        });
    case 'cleanup':
      if (restartingNow(s)) return note('再起動しています。終わってから操作してください', true);
      return openCleanupDialog(s);
  }
}

/* ── Restarting a running session on its conversation ── */
/* What a restart would cut off, as sentences for the dialog. Unread inbox items are not here:
   the new hub reads them again, so they are information, not a loss. */
function restartWarnings(s) {
  const out = [];
  if (s.waiting) {
    out.push(`確認待ち（${kindOf(s.waiting.kind)[0]}${s.waiting.title ? `: ${s.waiting.title}` : ''}）があります。gate は残り再起動後も答えられますが、待っている途中の処理は中断されます。`);
  }
  if (s.present && sessionActivity(s) === 'working') {
    out.push(`直近 1 分以内に出力があり、作業の途中かもしれません${s.phase ? `（フェーズ: ${s.phase}）` : ''}。実行中のコマンドや書きかけの返答は中断されます。`);
  }
  return out;
}

function fillWarnings(id, items) {
  const el = document.getElementById(id);
  el.hidden = !items.length;
  el.innerHTML = items.map(t => `<li class="warn">${esc(t)}</li>`).join('');
}

let restartTarget = null;
function openRestartDialog(s) {
  const why = restartWhy(s);
  if (why) return note(`セッションを再起動できません: ${why}`, true);
  restartTarget = s;
  const name = baseName(s.worktree) || s.id;
  sessEl('sess-restart-lead').textContent = `${name} の worker を閉じて、同じ worktree で同じ会話を再開します（開き直す tmux のウィンドウでは adj worker --resume --worktree ${s.worktree} が動きます）。`;
  fillWarnings('sess-restart-warnings', restartWarnings(s));
  sessEl('sess-restart-note').textContent = 'worktree・outbox・gate の記録は残り、再開した worker は outbox を確認して続きから進めます。';
  const dialog = sessEl('sess-restart-dialog');
  dialog.returnValue = '';
  dialog.showModal();
}
sessEl('sess-restart-dialog').addEventListener('close', e => {
  const s = restartTarget;
  restartTarget = null;
  if (e.target.returnValue === 'restart' && s) sessRestart(s);
});

/* Close the worker and open it again on the same conversation. The session counts as
   restarting from the request until a process other than the old one shows (`restartPending`),
   and for no longer than RESTART_MS; the buttons that would act on the old one stay held. */
async function sessRestart(s) {
  const key = sessionKey(s);
  const label = `セッションを再起動 (${key})`;
  const mark = restartKey(s);
  if (restartingNow(s)) return note('再起動しています。終わってから操作してください', true);
  return sessAct(`restart-${mark}`, label, async () => {
    // The pid of what runs now, not of the session as it was when the dialog opened: the
    // worker may have been restarted by hand in between, and the old pid would read as new.
    const current = (state.sessions || []).find(x => x.id === s.id && x.kind === s.kind) || s;
    const was = { pid: current.pid ?? null };
    sessRestarting.set(mark, { ...was, at: Date.now() });
    renderSessionsView();
    renderTaskPanel();
    showSessNotice('セッションを再起動しています…');
    try {
      const data = await api(`/api/sessions/${enc(s.id)}/restart`, { method: 'POST', body: '{}' });
      sessRestarting.set(mark, { ...was, at: Date.now() });
      note(label, false, data.description);
      showSessNotice('再起動しました' + (data.hubRunning === false ? '。hub は止まっています' : ''));
      if (panelTerm.sessionId === s.id) panelTerm.reconnect = true;
      await refresh();
    } catch (e) {
      sessRestarting.delete(mark);
      showSessNotice(`セッションを再起動できませんでした: ${e.message}`, true);
      note(`${label} → ${e.message}`, true);
      // The window may have been closed before the start failed.
      await refresh();
    } finally {
      renderSessionsView();
      renderTaskPanel();
    }
  });
}

/* ── Clean up a worktree ── */
let cleanupTarget = null;

function cleanupFactsHtml(s, git) {
  const items = [];
  if (s.present) items.push(['ok', 'セッションが動いています。片付けると閉じます']);
  const u = git.uncommitted || {};
  if (u.files > 0) items.push(['warn', `未コミットの変更: ${u.files} ファイル（+${u.insertions} -${u.deletions}）`]);
  if (u.untracked > 0) items.push(['warn', `追跡されていないファイル: ${u.untracked} 件`]);
  const up = git.unpushed || {};
  if (up.count > 0) {
    const commits = (up.commits || []).slice(0, 5).map(c => `${c.sha} ${c.subject}`).join('、');
    items.push(['warn', `push されていないコミット: ${up.count} 件${commits ? `（${commits}）` : ''}`]);
  }
  const m = git.merged || {};
  items.push(['ok', m.merged === true ? `${m.ref || m.base} にマージ済みです`
    : m.merged === false ? `${m.ref || m.base || 'ベース'} にはマージされていません`
      : `マージ済みかは判定できません${m.reason ? `（${m.reason}）` : ''}`]);
  if (!items.some(([k]) => k === 'warn')) items.unshift(['ok', '失われる作業はありません']);
  return items.map(([k, t]) => `<li class="${k}">${esc(t)}</li>`).join('');
}

async function openCleanupDialog(s) {
  if (restartingNow(s)) return note('再起動しています。終わってから操作してください', true);
  const name = baseName(s.worktree) || s.id;
  cleanupTarget = { s, name };
  sessEl('cleanup-name').textContent = name;
  sessEl('cleanup-path').textContent = s.worktree;
  sessEl('cleanup-force-name').textContent = name;
  sessEl('cleanup-confirm').placeholder = name;
  sessEl('cleanup-confirm').value = '';
  sessEl('cleanup-force-run').disabled = true;
  sessEl('cleanup-run').disabled = false;
  const force = sessEl('cleanup-force');
  force.hidden = true;
  force.open = false;
  cleanupError('');
  sessEl('cleanup-facts').innerHTML = '<li>確認しています…</li>';
  const dialog = sessEl('cleanup-dialog');
  dialog.returnValue = '';
  dialog.showModal();
  try {
    const git = await api(`/api/sessions/${enc(s.id)}/git`);
    if (cleanupTarget?.s !== s) return;
    sessEl('cleanup-facts').innerHTML = cleanupFactsHtml(s, git);
  } catch (e) {
    if (cleanupTarget?.s !== s) return;
    sessEl('cleanup-facts').innerHTML = `<li class="warn">${esc(`git の状態を確認できませんでした: ${e.message}`)}</li>`;
  }
}

function cleanupError(text) {
  const el = sessEl('cleanup-error');
  el.hidden = !text;
  el.textContent = text || '';
}

async function runCleanup(forced) {
  const t = cleanupTarget;
  if (!t || sessBusy.has('cleanup')) return;
  const line = `worktree を片付ける (${t.name})` + (forced ? ' 強制' : '');
  sessBusy.add('cleanup');
  sessEl('cleanup-run').disabled = true;
  sessEl('cleanup-force-run').disabled = true;
  cleanupError('');
  try {
    const data = await api(`/api/sessions/${enc(t.s.id)}/cleanup`, {
      method: 'POST', body: JSON.stringify(forced ? { force: true, confirm: sessEl('cleanup-confirm').value } : {}),
    });
    if (!data.removed) {
      const reasons = (data.reasons || []).map(r => `<li class="warn">${esc(r.detail)}</li>`).join('');
      sessEl('cleanup-facts').innerHTML = (data.git ? cleanupFactsHtml(t.s, data.git) : '') + reasons;
      cleanupError('失われる作業があるため、削除しませんでした。' + (data.closed ? 'セッションは閉じました。' : ''));
      const force = sessEl('cleanup-force');
      force.hidden = false;
      force.open = true;
      note(line, false, '失われる作業があるため削除しませんでした');
      return;
    }
    sessEl('cleanup-dialog').close('done');
    const branch = data.branch?.name
      ? (data.branch.deleted ? `ブランチ ${data.branch.name} も削除しました` : `ブランチ ${data.branch.name} は消せませんでした: ${data.branch.error}`) : '';
    const badHooks = (data.hooks || []).filter(h => !h.ok).length;
    const text = ['worktree を片付けました', branch, (data.tasks || []).length ? `タスク ${data.tasks.length} 件を完了にしました` : '',
      badHooks ? `onWorktreeRemove が ${badHooks} 件失敗しました` : '', (data.taskErrors || []).length ? 'タスクの更新に失敗したものがあります' : '']
      .filter(Boolean).join('。');
    const bad = !!branch && !data.branch.deleted || badHooks > 0 || (data.taskErrors || []).length > 0;
    note(line, bad, text);
    showSessNotice(text, bad);
    await refresh();
  } catch (e) {
    cleanupError(e.message);
    note(line, true, e.message);
  } finally {
    sessBusy.delete('cleanup');
    sessEl('cleanup-run').disabled = false;
    sessEl('cleanup-force-run').disabled = sessEl('cleanup-confirm').value !== cleanupTarget?.name;
  }
}
sessEl('cleanup-run').addEventListener('click', () => runCleanup(false));
sessEl('cleanup-force-run').addEventListener('click', () => runCleanup(true));
sessEl('cleanup-confirm').addEventListener('input', e => {
  sessEl('cleanup-force-run').disabled = !cleanupTarget || e.target.value !== cleanupTarget.name;
});
// Enter in the box would submit the dialog's own form, which closes it.
sessEl('cleanup-confirm').addEventListener('keydown', e => {
  if (e.key !== 'Enter' || e.isComposing || e.keyCode === 229) return;
  e.preventDefault();
  if (cleanupTarget && e.target.value === cleanupTarget.name) runCleanup(true);
});
sessEl('cleanup-dialog').addEventListener('close', () => { cleanupTarget = null; });

/* Start the hub of a group: from the board it is on, which for 「すべて」 is not this page's. */
function startGroupHub(g) {
  if (!scopeAll()) return hubStart(g.id);
  const row = hubBoardOf(g);
  const slug = row?.slug || g.slug;
  return hubStartAt(`/b/${slug}`, g.hub.id, g.hub.key, row).then(() => refresh(true));
}

/* A hub's terminal is the task panel's, over the list; 「すべて」 shows the hub's board first. */
function openGroupHub(g) {
  const ref = HUB_REF + g.id;
  if (!scopeAll()) return openTaskPanel(ref, 'term');
  go({ board: hubBoardOf(g)?.slug || g.slug, view: 'sessions', task: ref, pane: 'term' });
}

sessEl('sess-groups').addEventListener('click', e => {
  const act = e.target.closest('[data-hub-act]');
  if (act) {
    if (act.disabled) return;
    const g = sessionGroups().find(x => x.gid === act.closest('.sess-group')?.dataset.gid);
    if (!g) return;
    if (act.dataset.hubAct === 'hub-start') return startGroupHub(g);
    return openGroupHub(g);
  }
  const row = e.target.closest('[data-sref]');
  if (row) openSessionRef(row.dataset.sref);
});
// `toggle` does not bubble, and the list is rebuilt: heard on the way down.
sessEl('sess-groups').addEventListener('toggle', e => {
  const gid = e.target.dataset?.orphans;
  if (gid == null) return;
  const key = `orphans:${gid}`;
  const set = new Set(prefs.sessionsFolded);
  // Drawing the list sets `open` too, which lands here with nothing to change.
  if (set.has(key) === e.target.open) return;
  if (e.target.open) set.add(key); else set.delete(key);
  prefs.sessionsFolded = [...set];
  savePrefs();
}, true);
sessEl('sess-notice').addEventListener('click', e => { if (e.target.closest('[data-sess-dismiss]')) showSessNotice(''); });
