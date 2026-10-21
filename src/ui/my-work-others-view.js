/* ── 「レビュー」: the list of the PRs others asked you to review, drawn ──────────────────────
   The list of the rail's 「レビュー」, drawn while `nav.list` is 'review'. It reads `GET /api/others` (one document for
   the server, as `/api/work` is) and syncs with GitHub only when asked (`POST /api/others/sync`). The pure half (what a
   row says, what is new, the order, the hub's row) is my-work-others.js; this file draws and keeps what the page holds:
   the document, the read marks in this browser (`adj.seenOthers`), and the sync in flight. Loaded after my-work.js, which
   calls into it at run time. */

const others = {
  doc: null,          // the document as last read
  fetchedMs: 0,       // when this page read it (Date.now()): the server's `now` is as of then
  error: null,        // the message of a read that failed; the document is then the last that was read
  busy: false,
  again: false,       // a forced read asked for while one was out
  syncing: false,     // a sync this page started is out
  syncError: null,    // why the last sync this page started could not be done
  sig: '',            // what the list was last drawn from
  asking: new Set(),  // ids whose AI 下読み this page asked for and the answer is not back
  closing: new Set(), // ids whose reader this page is closing and the answer is not back
  starting: false,    // the code-review hub's start this page asked for is out
  stopping: false,    // the code-review hub's stop this page asked for is out
  stopAsked: false,   // the confirmation of stopping the code-review hub is open (the dialog is shared with the repositories' hubs)
};

/* The server's clock now, as far as this page can tell: the document's `now` and the time since it was read. */
const othersNowEst = () => others.doc ? others.doc.now + (Date.now() - others.fetchedMs) / 1000 : Date.now() / 1000;
const othersRecords = () => others.doc?.records || [];
const othersById = id => othersRecords().find(r => r.id === id) || null;

/* The marks of this browser, read from storage every time so that what another tab wrote is what is read. */
function othersMarks() {
  try { return workParseMarks(localStorage.getItem(OTHERS_SEEN_KEY) || ''); } catch { return {}; }
}

/* Write `patches` ([id, { read }]) onto the marks. The marks are read again first and a time only moves later, so another
   tab's write is not undone; the marks of records long gone are dropped, unless the last read failed (a failed read is not
   absence). A mark covers what the document showed (`now` of it) and nothing that came after. */
function othersWriteMarks(patches) {
  let marks = othersMarks();
  for (const [id, patch] of patches) marks = workMarkMerge(marks, id, patch);
  if (others.doc && !others.error) marks = workPruneMarks(marks, new Set(othersRecords().map(r => r.id)), others.doc.now);
  try { localStorage.setItem(OTHERS_SEEN_KEY, JSON.stringify(marks)); } catch {}
  // The `storage` event is for the other tabs.
  if (view === 'work') renderOthersList();
  renderWorkBadge();
}

/* How many records are new, whatever the list is narrowed to. Nothing before the first read; a failed read keeps the last. */
const othersNewCount = () => others.doc ? othersNewIds(othersRecords(), othersMarks()).length : 0;

/* The dot on the switch's 「AI 下読み」: the last sync is more than 30 minutes old. */
function renderOthersStaleDot() {
  const dot = wk('ov-stale-dot');
  if (dot) dot.hidden = !(others.doc && othersStale(others.doc.lastSync, othersNowEst()));
}

async function refreshOthers(force = false) {
  if (!multiBoard) return;
  if (others.busy) {
    // An answer to a sync just done must not be lost to the read already out, which may predate it.
    if (force) others.again = true;
    return;
  }
  others.busy = true;
  try {
    const doc = await boardApi('', '/api/others');
    others.doc = doc;
    others.fetchedMs = Date.now();
    others.error = null;
  } catch (e) {
    // The list stays as it was: a failed read is not an empty list, and does not zero the count.
    others.error = e.message;
  } finally {
    others.busy = false;
  }
  if (others.again) {
    others.again = false;
    refreshOthers(true);
  }
  renderWorkBadge();
  renderOthersStaleDot();
  renderOthersCount();
  if (view === 'work') renderOthersList(force);
  // The review list's empty middle says whether there is anything to choose from: drawn again when the records come or change.
  if (view === 'work' && nav.list === 'review' && !nav.task) renderWorkTerm();
  // The PR that is open follows its record, and one named by the address is opened once the records are here.
  prFollow();
}

/* The rail's レビュー count: the records that wait on the person, in the selected organisation. */
function renderOthersCount() {
  const badge = document.getElementById('review-count');
  if (!badge) return;
  const n = othersWaiting(othersRecords(), todoInScope()).length;
  badge.textContent = n;
  badge.classList.toggle('zero', n === 0);
}
// Another organisation was selected: the count is of that one.
onOrgChange(renderOthersCount);

/* Open the PR in the panel and the middle, as a task opens (pr-panel.js); selecting a row does not read it. */
function othersSelect(rec) {
  prOpen(rec);
}

/* ── sync ── */

/* The click on a notification of review requests: this page, on the 「レビュー」 list. */
function openReviewNotified(notification) {
  try { notification?.close(); } catch {}
  window.focus();
  go({ board: 'all', view: 'work', list: 'review', task: null, pane: 'detail' });
}

/* One desktop notification for what a sync brought in, from the tab that synced; the tag makes the next sync's replace it. */
function ringReviewRequests(res) {
  const content = notifyReviewContent(res.arrived, res.records);
  if (!content || !prefs.notify.review || !(window.Notification && Notification.permission === 'granted')) return;
  const n = new Notification(content.title, { body: content.body, tag: `adj-review-${res.lastSync?.at || ''}` });
  n.onclick = () => openReviewNotified(n);
}

async function syncOthers() {
  if (!multiBoard || others.syncing) return;
  others.syncing = true;
  others.syncError = null;
  renderOthersList();
  let busyElsewhere = false;
  let res = null;
  try {
    res = await boardApi('', '/api/others/sync', { method: 'POST' });
  } catch (e) {
    // Another process is syncing: not a failure, and what it reads is what the next read shows.
    if (e.status === 409) {
      busyElsewhere = true;
      note('adj others sync', false, 'ほかで同期しています');
    } else others.syncError = e.message;
  } finally {
    others.syncing = false;
  }
  // The button stops saying 「同期中…」 now, whether or not a read is out to carry the answer.
  if (view === 'work') renderOthersList();
  // A notification that cannot be shown does not make a sync that worked a failed one.
  if (res) { try { ringReviewRequests(res); } catch {} }
  // The sync stands when a reader of a merged or closed PR could not be closed; it is tried again at the next one.
  if (res?.closeWarning) note('adj others sync', true, res.closeWarning);
  await refreshOthers(true);
  if (busyElsewhere) setTimeout(() => refreshOthers(true), 2000);
}

/* ── AI 下読み and the code-review hub ── */

/* Ask for a read-through of one PR, or with `mode` 'split' for a split proposal. One request per PR at a time; the answer is read
   back either way, since a refusal (the hub stopped, the PR done) is also something the list now shows. */
async function othersRequestAi(id, mode) {
  if (!multiBoard || others.asking.has(id)) return;
  others.asking.add(id);
  renderOthersList(true);
  prRedraw();
  try {
    const res = await boardApi('', '/api/others/ai', { method: 'POST', body: JSON.stringify(mode ? { id, mode } : { id }) });
    // The request stands; this is what the queue could not do after it.
    if (res?.warning) note('adj review request', true, res.warning);
  } catch (e) {
    note('adj review request', true, e.message);
  } finally {
    others.asking.delete(id);
    prRedraw();
  }
  await refreshOthers(true);
}

/* Close the reader of one PR: its window goes, and the read-through, the drafts and the record stay. One close per PR at a time;
   the answer is read back either way, since a refusal (no reader any more) is also something the list now shows. */
async function othersCloseReader(id) {
  if (!multiBoard || others.closing.has(id)) return;
  others.closing.add(id);
  prRedraw();
  try {
    const res = await boardApi('', '/api/others/reader/close', { method: 'POST', body: JSON.stringify({ id }) });
    note('下読みを閉じる', false, '閉じました');
    // The reader is closed; this is what the queue could not do after a running read gave its place up.
    if (res?.warning) note('下読みを閉じる', true, res.warning);
  } catch (e) {
    note('下読みを閉じる', true, e.message);
  } finally {
    // Not before the records are read again: the button would be on again against the record as it was.
    try {
      await refreshOthers(true);
      // A task's own document says the same, and its panel must not offer the button against the old one.
      if (typeof taskReadRefresh === 'function') await taskReadRefresh(true);
    } finally {
      others.closing.delete(id);
      prRedraw();
    }
  }
}

/* Start the code-review hub in a new window. `starting` stays on until the records are read again (the task's document too, unless
   a read of it is already out), which narrows the time a start button is offered against a hub that is already up. */
async function othersStartHub() {
  if (!multiBoard || others.starting) return;
  others.starting = true;
  renderOthersList(true);
  prRedraw();
  try {
    const res = await boardApi('', '/api/others/hub/start', { method: 'POST', body: JSON.stringify({}) });
    if (res.started === false) note('adj hub --review', false, 'すでに動いています');
  } catch (e) {
    note('adj hub --review', true, e.message);
  } finally {
    try {
      await refreshOthers(true);
      if (typeof taskReadRefresh === 'function') await taskReadRefresh(true);
    } finally {
      others.starting = false;
      renderOthersList(true);
      prRedraw();
    }
  }
}

/* Ask whether to stop the code-review hub, in the dialog the repositories' hubs use (`openHubStopDialog` in actions.js fills it
   for those). Its `close` listener below stops the hub when it was confirmed. */
function othersOpenHubStop() {
  if (!multiBoard || others.stopping) return;
  if (others.starting) return note('hub を止める', true, 'hub を起動しています。起動してから止めてください');
  const text = othersHubStopText(othersHubView(others.doc?.hub, othersRecords()));
  document.getElementById('hub-stop-heading').textContent = 'hub を止める';
  document.getElementById('hub-stop-confirm').textContent = 'hub を止める';
  document.getElementById('hub-stop-icon').textContent = 'stop_circle';
  document.getElementById('hub-stop-submit').className = 'btn-m3-danger';
  fillWarnings('hub-stop-warnings', text.warnings);
  document.getElementById('hub-stop-lead').textContent = text.lead;
  document.getElementById('hub-stop-note').textContent = text.note;
  const dialog = document.getElementById('hub-stop-dialog');
  others.stopAsked = true;
  dialog.returnValue = '';
  dialog.showModal();
}
// The dialog is shared with actions.js's listener, which does nothing unless it was opened for a repository's hub.
document.getElementById('hub-stop-dialog').addEventListener('close', e => {
  if (!others.stopAsked) return;
  others.stopAsked = false;
  if (e.target.returnValue === 'stop') othersStopHub();
});

/* Stop the code-review hub (its pane closes). The records, the read-throughs and the queue stay, and so do the readers that run.
   The records are read again before the button is drawn, since it would be offered against the hub as it was. */
async function othersStopHub() {
  if (!multiBoard || others.stopping) return;
  others.stopping = true;
  renderOthersList(true);
  try {
    const res = await boardApi('', '/api/others/hub/stop', { method: 'POST', body: JSON.stringify({}) });
    note('tmux kill-pane (hub コードレビュー)', false, res.wasRunning ? 'hub を止めました' : 'hub はすでに止まっていました（記録を片付けました）');
  } catch (e) {
    note('tmux kill-pane (hub コードレビュー)', true, e.message);
  } finally {
    try {
      await refreshOthers(true);
      if (typeof taskReadRefresh === 'function') await taskReadRefresh(true);
    } finally {
      others.stopping = false;
      renderOthersList(true);
      prRedraw();
    }
  }
}

/* ── the list ── */

const OTHERS_SYNC_WHY = 'AI 下読みはまだ始められません';

/* A glyph of a state, as the list of tasks draws its own. */
const othersGlyphHtml = rec => {
  const [icon, label, cls] = othersStateOf(rec);
  return `<span class="material-symbols-outlined wk-glyph ${cls}" role="img" aria-label="${esc(label)}" title="${esc(label)}">${icon}</span>`;
};

function othersRowHtml(rec, isNew, hubView, nowEst, records) {
  const line = othersStateLine(rec, records, nowEst);
  const when_ = othersRequestedLabel(rec, nowEst);
  const author = rec.author ? `@${esc(rec.author)}` : '（削除されたアカウント）';
  const ai = othersAiView(rec, records, nowEst);
  const stage = othersStageOfKind(ai.kind);
  const meta = [
    `<span class="ov-stage ov-stage-${stage}">${OTHERS_STAGE_MARK[stage]}</span>`,
    `#${esc(rec.number)}`,
    author,
    `<span class="add">+${Number(rec.additions) || 0}</span> <span class="del">-${Number(rec.deletions) || 0}</span>`,
    `${Number(rec.changedFiles) || 0} files`,
    `<span class="ov-size" title="変更の大きさ（追加と削除の行数の合計）">${othersSize(rec)}</span>`,
    when_ ? `<span title="${esc(when(rec.requestedAt))}">${esc(when_)}</span>` : '',
  ].filter(Boolean).map(x => `<span>${x}</span>`).join('');
  // A PR too large to read is offered 「分割案」 where it would be offered 「AI 下読み」, and 「AI 下読み」 is not offered on its row.
  const oor = OTHERS_LIVE.includes(rec.state) ? othersOutOfReview(rec) : null;
  const asking = others.asking.has(rec.id);
  // Disabled for good while a read cannot be asked for (the hub is down, or no reader can be opened), with why as the title; while a request is out it only ignores the click.
  const askAttrs = `${hubView.canRead ? '' : ` disabled title="${esc(hubView.why || OTHERS_SYNC_WHY)}"`}${asking ? ' aria-disabled="true"' : ''}`;
  // What a button asks for follows whether the PR is out of review now (`othersAiView` words it so), not what was asked before.
  // The actions are described once: drawn for real beside the row button (a button cannot hold buttons) and again, invisible, at the end of the state line, which reserves their room there.
  const acts = [];
  if (ai.action) {
    const [name, icon, label] = oor ? ['split', 'call_split', '分割案'] : ['ai', 'menu_book', ai.action];
    acts.push({ cls: 'ov-act', attrs: ` data-ov-${name}="${esc(rec.id)}"${askAttrs}`, html: `<span class="material-symbols-outlined" aria-hidden="true">${icon}</span>${esc(asking ? '依頼中…' : label)}` });
  }
  if (isNew) acts.push({ cls: 'wk-act', attrs: ` data-ov-read="${esc(rec.id)}" aria-label="既読にする" title="既読にする"`, html: '<span class="material-symbols-outlined" aria-hidden="true">done</span>' });
  const realActs = acts.map(a => `<button type="button" class="${a.cls}"${a.attrs}>${a.html}</button>`).join('');
  const ghostActs = acts.length ? `<span class="ov-acts-ghost" aria-hidden="true">${acts.map(a => `<span class="${a.cls}">${a.html}</span>`).join('')}</span>` : '';
  const stateLine = line.text || acts.length
    ? `<span class="wk-line ov-line${line.cls ? ` ${line.cls}` : ''}${line.text ? '' : ' ov-line-bare'}">${esc(line.text)}${ghostActs}</span>` : '';
  const dim = othersStateOf(rec)[2] === 'ov-dim' ? ' dim' : '';
  const selected = nav.task === PR_REF + rec.id;
  return `<div class="wk-band-row ov-row-wrap" data-ov-cell="${esc(rec.id)}">
    <button type="button" class="wk-row ov-row${dim}${isNew ? ' new' : ''}${selected ? ' selected' : ''}" data-ov="${esc(rec.id)}"${selected ? ' aria-current="true"' : ''} title="開く">
      ${othersGlyphHtml(rec)}<span class="wk-main">
        <span class="wk-title">${isNew ? '<span class="ov-new">新着</span>' : ''}${rec.requested ? '<span class="ov-req">依頼</span>' : ''}${esc(rec.title)}</span>
        <span class="wk-meta">${meta}</span>
        ${oor ? `<span class="wk-line ov-oor-line">${esc(othersOutOfReviewLine(oor))}</span>
        ` : ''}${stateLine}
      </span></button>${realActs ? `<span class="wk-acts ov-acts">${realActs}</span>` : ''}
  </div>`;
}

/* The code-review hub's row. Not a button: it opens nothing here; its own button starts it when it is stopped and can be started, and
   stops it (after asking) when it runs and can be stopped. */
function othersHubHtml(hubView) {
  const stopped = hubView.state === 'stopped';
  const hubActHtml = hubView.startable
    ? `<span class="wk-acts"><button type="button" class="ov-act" data-ov-hub-start${others.starting ? ' aria-disabled="true"' : ''}>${others.starting ? '起動中…' : 'hub を起動'}</button></span>`
    : hubView.stoppable
      ? `<span class="wk-acts"><button type="button" class="ov-act" data-ov-hub-stop${others.stopping ? ' aria-disabled="true"' : ''}>${others.stopping ? '止めています…' : 'hub を止める'}</button></span>` : '';
  return `<div class="wk-row ov-hub${stopped ? ' dim' : ''}" data-ov-hub>
    <span class="material-symbols-outlined wk-glyph ${stopped ? 'ov-dim' : 'ov-ready'}" aria-hidden="true">smart_toy</span><span class="wk-main">
      <span class="wk-title"><span class="wk-tag">hub</span>コードレビュー</span>
      <span class="wk-meta"><span>AI 下読み ${hubView.running} / ${hubView.max}${hubView.queued ? ` · 順番待ち ${hubView.queued}` : ''}</span><span>${esc(stopped ? '止まっています' : hubView.state === 'running' ? '動いています' : hubView.state)}</span></span>
      ${hubView.why ? `<span class="wk-line">${esc(hubView.why)}</span>` : ''}
    </span>${hubActHtml}
  </div>`;
}

/* The sync button: its words are the time of the last sync; while one is out it says so and does nothing (aria-disabled, so the
   keyboard stays where it is). It is highlighted when the last sync is old. */
function othersSyncButtonHtml(lastSync, nowEst, stale, label = '', lite = false) {
  const at = stampSecs(lastSync?.at);
  const words = others.syncing ? '同期中…' : label || othersSyncWords(lastSync, nowEst, false);
  const tip = at == null ? '' : `最後の同期: ${when(lastSync.at)}${stale ? '（30 分以上たっています）' : ''}`;
  return `<button type="button" class="ov-sync-btn${stale ? ' stale' : ''}${lite ? ' lite' : ''}" data-ov-sync${others.syncing ? ' aria-disabled="true"' : ''}${tip ? ` title="${esc(tip)}"` : ''}><span class="material-symbols-outlined" aria-hidden="true">sync</span>${esc(words)}</button>`;
}

/* What the last sync could not do, one line each. */
function othersNotes(lastSync) {
  const out = [];
  const failedAfter = lastSync?.error && lastSync.errorAt && (!lastSync.at || lastSync.errorAt > lastSync.at) ? lastSync.error : null;
  const why = others.syncError || failedAfter;
  if (why) out.push({ alert: true, text: `前回の同期に失敗しました: ${why}` });
  if (lastSync?.complete === false) {
    const n = (lastSync.failed || []).length;
    out.push({ text: n ? `${n} 件を読めませんでした` : '一部を読めませんでした' });
  }
  if (lastSync?.truncated) out.push({ text: '100 件を超えたため一部だけです' });
  return out;
}

function othersListHtml() {
  const doc = others.doc;
  if (!doc) return `<div class="wk-empty">${esc(others.error ? `読み込めませんでした: ${others.error}` : '読み込み中…')}</div>`;
  const nowEst = othersNowEst();
  const records = doc.records || [];
  const lastSync = doc.lastSync;
  const stale = othersStale(lastSync, nowEst);
  const hideDone = othersHidesDone(prefs.othersHideDone);
  const shown = othersShown(records, hideDone);
  const repo = othersEffectiveRepo(shown, prefs.othersRepo);
  const stage = othersEffectiveStage(prefs.othersStage);
  const marks = othersMarks();
  const hubView = othersHubView(doc.hub, records);
  const never = !lastSync?.at;

  const chips = records.length ? [{ key: '', label: 'すべて' }, ...othersRepoChips(shown)] : [];
  const chipsHtml = chips.length
    ? `<div class="ov-chips" role="group" aria-label="リポジトリで絞り込む">${chips.map(c => `<button type="button" class="ov-chip" data-ov-repo="${esc(c.key)}" aria-pressed="${c.key === repo}">${esc(c.label)}</button>`).join('')}</div>` : '';
  const stageChips = records.length ? othersStageChips(shown, repo, nowEst) : [];
  const stageChipsHtml = stageChips.length
    ? `<div class="ov-chips" role="group" aria-label="AI 下読みで絞り込む">${stageChips.map(c => `<button type="button" class="ov-chip" data-ov-stage="${esc(c.key)}" aria-pressed="${c.key === stage}">${esc(c.label)} <span class="ov-chip-count">${c.count}</span></button>`).join('')}</div>` : '';
  // The toggle stays while everything is hidden, since it is what brings the rows back.
  const doneToggleHtml = records.length
    ? `<button type="button" class="ov-chip" data-ov-done aria-pressed="${hideDone}" title="マージ・クローズ・取り下げ・承認で終わった PR を隠します">終わったものを除く</button>` : '';
  const alerts = [
    ...(others.error ? [{ alert: true, text: '最新の状態を読めませんでした（表示は前回のものです）' }] : []),
    ...othersNotes(lastSync),
  ].map(n => n.alert ? `<div class="wk-notice" role="alert">${esc(n.text)}</div>` : `<div class="ov-note">${esc(n.text)}</div>`).join('');

  let body;
  if (never && !records.length) {
    body = `<div class="wk-empty">あなたにレビューを頼んでいる PR は、同期すると読み込みます<br>${othersSyncButtonHtml(lastSync, nowEst, false, 'いま同期する', true)}</div>`;
  } else if (!records.length) {
    body = '<div class="wk-empty">あなたにレビューを頼んでいる PR はありません</div>';
  } else {
    const groups = othersGroups(shown, repo, stage, nowEst);
    if (!groups.length) {
      // Counted over all the records with the repository that applies above, so the number is of what the toggle would bring back.
      const hiddenN = hideDone ? othersGroups(records, repo, stage, nowEst).reduce((n, g) => n + g.rows.length, 0) : 0;
      const showBtn = '<button type="button" class="ov-sync-btn lite" data-ov-done><span class="material-symbols-outlined" aria-hidden="true">visibility</span>終わったものも表示</button>';
      const label = stage ? stageChips.find(c => c.key === stage)?.label || '' : '';
      const text = stage ? `「${label}」の PR はありません` : '終わっていない PR はありません';
      body = `<div class="wk-empty">${esc(text)}${hiddenN > 0 ? `。終わった PR ${hiddenN} 件を隠しています<br>${showBtn}` : ''}</div>`;
    } else body = groups.map(g => `<section class="wk-group" aria-label="${esc(g.label)}">
      <div class="wk-head repo"><span class="wk-head-main"><span class="wk-head-name">${esc(g.label)}${g.unregistered ? ' <span class="ov-unreg" title="adj に登録していないリポジトリです（同じオーナーなので依頼だけ出ます）">未登録</span>' : ''}</span></span><span class="wk-count">${g.rows.length}</span></div>
      <div class="wk-body">${g.rows.map(r => othersRowHtml(r, othersSeenClass(r, marks[r.id] || {}) === 'new', hubView, nowEst, records)).join('')}</div>
    </section>`).join('');
  }
  // Every new row counts, the ones the repository and AI 下読み chips hide too (as 「タスク」 marks every new entry whatever is folded).
  const readAllHtml = othersNewIds(records, marks).length ? workReadAllButtonHtml('data-ov-read-all') : '';
  return `${othersHubHtml(hubView)}<div class="ov-sync-row">${othersSyncButtonHtml(lastSync, nowEst, stale)}${chipsHtml}${stageChipsHtml}${doneToggleHtml}${readAllHtml}</div>${alerts}${body}`;
}

/* How to find again what has the keyboard focus after a redraw: the control by its own data, else (a button that has gone, as
   既読 does once pressed) the row it was on. */
function othersHeldSel(el) {
  const d = el?.dataset || {};
  const q = (name, v) => `[data-${name}="${CSS.escape(v)}"]`;
  const rowOf = id => q('ov', id);
  if (d.ov != null) return [rowOf(d.ov)];
  if (d.ovRead != null) return [q('ov-read', d.ovRead), rowOf(d.ovRead)];
  if (d.ovAi != null) return [q('ov-ai', d.ovAi), rowOf(d.ovAi)];
  if (d.ovHubStart != null) return ['[data-ov-hub-start]', '[data-ov-hub-stop]', '[data-ov-hub]'];
  if (d.ovHubStop != null) return ['[data-ov-hub-stop]', '[data-ov-hub-start]', '[data-ov-hub]'];
  if (d.ovSplit != null) return [q('ov-split', d.ovSplit), rowOf(d.ovSplit)];
  if (d.ovRepo != null) return [q('ov-repo', d.ovRepo)];
  if (d.ovStage != null) return [q('ov-stage', d.ovStage)];
  if (d.ovDone != null) return ['[data-ov-done]'];
  if (d.ovReadAll != null) return ['[data-ov-read-all]', '.ov-sync-row [data-ov-sync]', '[data-ov-sync]'];
  if (d.ovSync != null) return ['.ov-sync-row [data-ov-sync]', '[data-ov-sync]'];
  return [];
}

function renderOthersList(force = false) {
  const root = wk('ov-groups');
  if (!root || nav.list !== 'review') return;
  const sig = JSON.stringify([others.doc ? othersSig(others.doc, othersNowEst()) : null, prefs.othersRepo, prefs.othersStage, prefs.othersHideDone, othersMarks(), others.error, others.syncing, others.syncError, [...others.asking], [...others.closing], others.starting, others.stopping, nav.task]);
  if (!force && sig === others.sig) return;
  others.sig = sig;
  const scroll = root.closest('.wk-list-scroll');
  const top = scroll.scrollTop;
  const at = document.activeElement;
  const held = root.contains(at) ? othersHeldSel(at) : [];
  root.innerHTML = othersListHtml();
  scroll.scrollTop = top;
  for (const sel of held) {
    const el = root.querySelector(sel);
    if (el) { el.focus({ preventScroll: true }); break; }
  }
}

wk('ov-groups').addEventListener('click', e => {
  let b;
  if ((b = e.target.closest('[data-ov-sync]'))) return syncOthers();
  if ((b = e.target.closest('[data-ov-repo]'))) {
    prefs.othersRepo = b.dataset.ovRepo;
    savePrefs();
    return renderOthersList();
  }
  if ((b = e.target.closest('[data-ov-stage]'))) {
    prefs.othersStage = othersEffectiveStage(b.dataset.ovStage);
    savePrefs();
    return renderOthersList();
  }
  if (e.target.closest('[data-ov-done]')) {
    prefs.othersHideDone = !othersHidesDone(prefs.othersHideDone);
    savePrefs();
    return renderOthersList();
  }
  if (e.target.closest('[data-ov-read-all]')) {
    if (!others.doc) return;
    const patches = othersReadAllPatches(othersRecords(), othersMarks(), others.doc.now);
    if (patches.length) othersWriteMarks(patches);
    return;
  }
  if ((b = e.target.closest('[data-ov-read]'))) {
    if (others.doc) othersWriteMarks([[b.dataset.ovRead, { read: others.doc.now }]]);
    return;
  }
  // A button that is disabled, or only marked so while its request is out, does nothing.
  if ((b = e.target.closest('[data-ov-ai]'))) {
    if (b.disabled || b.getAttribute('aria-disabled') === 'true') return;
    return othersRequestAi(b.dataset.ovAi);
  }
  if ((b = e.target.closest('[data-ov-hub-start]'))) {
    if (b.getAttribute('aria-disabled') === 'true') return;
    return othersStartHub();
  }
  if ((b = e.target.closest('[data-ov-split]'))) {
    if (b.disabled || b.getAttribute('aria-disabled') === 'true') return;
    return othersRequestAi(b.dataset.ovSplit, 'split');
  }
  if ((b = e.target.closest('[data-ov-hub-stop]'))) {
    if (b.getAttribute('aria-disabled') === 'true') return;
    return othersOpenHubStop();
  }
  // The hub's row opens nothing.
  if (e.target.closest('[data-ov-hub]')) return;
  if ((b = e.target.closest('.ov-row[data-ov]'))) {
    const rec = othersById(b.dataset.ov);
    if (rec) othersSelect(rec);
  }
});
