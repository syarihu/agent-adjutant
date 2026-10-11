/* ── いまの仕事 「AI 下読み」: the list of the PRs others asked you to review, drawn ──────────────────────
   The second list of 「いまの仕事」, chosen by the switch in its heading row. It reads `GET /api/others` (one document for
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
  starting: false,    // the code-review hub's start this page asked for is out
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
  if (view === 'work') renderOthersList(force);
  // The PR that is open follows its record, and one named by the address is opened once the records are here.
  prFollow();
}

/* Open the PR in the panel and the middle, as a task opens (pr-panel.js); selecting a row does not read it. */
function othersSelect(rec) {
  prOpen(rec);
}

/* ── sync ── */

/* The click on a notification of review requests: this page, on the AI 下読み list. */
function openReviewNotified(notification) {
  try { notification?.close(); } catch {}
  window.focus();
  prKeepTab();
  prefs.workTab = 'others';
  savePrefs();
  go({ board: 'all', view: 'work', task: null, pane: 'detail' });
  renderWorkTabs();
  renderOthersList();
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
  await refreshOthers(true);
  if (busyElsewhere) setTimeout(() => refreshOthers(true), 2000);
}

/* ── AI 下読み and the code-review hub ── */

/* Ask for a read-through of one PR. One request per PR at a time; the answer is read back either way, since a refusal (the hub
   stopped, the PR done) is also something the list now shows. */
async function othersRequestAi(id) {
  if (!multiBoard || others.asking.has(id)) return;
  others.asking.add(id);
  renderOthersList(true);
  prRedraw();
  try {
    const res = await boardApi('', '/api/others/ai', { method: 'POST', body: JSON.stringify({ id }) });
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

/* Start the code-review hub in a new window. */
async function othersStartHub() {
  if (!multiBoard || others.starting) return;
  others.starting = true;
  renderOthersList(true);
  try {
    const res = await boardApi('', '/api/others/hub/start', { method: 'POST', body: JSON.stringify({}) });
    if (res.started === false) note('adj hub --review', false, 'すでに動いています');
  } catch (e) {
    note('adj hub --review', true, e.message);
  } finally {
    others.starting = false;
  }
  await refreshOthers(true);
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
  const meta = [
    `#${esc(rec.number)}`,
    author,
    `<span class="add">+${Number(rec.additions) || 0}</span> <span class="del">-${Number(rec.deletions) || 0}</span>`,
    `${Number(rec.changedFiles) || 0} files`,
    `<span class="ov-size" title="変更の大きさ（追加と削除の行数の合計）">${othersSize(rec)}</span>`,
    when_ ? `<span title="${esc(when(rec.requestedAt))}">${esc(when_)}</span>` : '',
  ].filter(Boolean).map(x => `<span>${x}</span>`).join('');
  const split = othersOutOfReview(rec);
  const ai = othersAiView(rec, records, nowEst);
  const asking = others.asking.has(rec.id);
  // Disabled for good while a read cannot be asked for (the hub is down, or no reader can be opened), with why as the title; while a request is out it only ignores the click.
  const aiButton = ai.action && !split
    ? `<button type="button" class="ov-act" data-ov-ai="${esc(rec.id)}"${hubView.canRead ? '' : ` disabled title="${esc(hubView.why || OTHERS_SYNC_WHY)}"`}${asking ? ' aria-disabled="true"' : ''}><span class="material-symbols-outlined" aria-hidden="true">menu_book</span>${esc(asking ? '依頼中…' : ai.action)}</button>` : '';
  const acts = [
    aiButton,
    split ? `<button type="button" class="ov-act" data-ov-split="${esc(rec.id)}"><span class="material-symbols-outlined" aria-hidden="true">call_split</span>分割案</button>` : '',
    isNew ? `<button type="button" class="wk-act" data-ov-read="${esc(rec.id)}" aria-label="既読にする" title="既読にする"><span class="material-symbols-outlined" aria-hidden="true">done</span></button>` : '',
  ].filter(Boolean).join('');
  const dim = othersStateOf(rec)[2] === 'ov-dim' ? ' dim' : '';
  const selected = nav.task === PR_REF + rec.id;
  return `<div class="wk-band-row ov-row-wrap" data-ov-cell="${esc(rec.id)}">
    <button type="button" class="wk-row ov-row${dim}${isNew ? ' new' : ''}${selected ? ' selected' : ''}" data-ov="${esc(rec.id)}"${selected ? ' aria-current="true"' : ''} title="開く">
      ${othersGlyphHtml(rec)}<span class="wk-main">
        <span class="wk-title">${isNew ? '<span class="ov-new">新着</span>' : ''}${esc(rec.title)}</span>
        <span class="wk-meta">${meta}</span>
        <span class="wk-line ov-line${line.cls ? ` ${line.cls}` : ''}">${esc(line.text)}</span>
      </span></button>${acts ? `<span class="wk-acts">${acts}</span>` : ''}
  </div>`;
}

/* The code-review hub's row. Not a button: it opens nothing here; its own button starts it when it is stopped and can be started. */
function othersHubHtml(hubView) {
  const stopped = hubView.state === 'stopped';
  return `<div class="wk-row ov-hub${stopped ? ' dim' : ''}" data-ov-hub>
    <span class="material-symbols-outlined wk-glyph ${stopped ? 'ov-dim' : 'ov-ready'}" aria-hidden="true">smart_toy</span><span class="wk-main">
      <span class="wk-title"><span class="wk-tag">hub</span>コードレビュー</span>
      <span class="wk-meta"><span>AI 下読み ${hubView.running} / ${hubView.max}${hubView.queued ? ` · 順番待ち ${hubView.queued}` : ''}</span><span>${esc(stopped ? '止まっています' : hubView.state === 'running' ? '動いています' : hubView.state)}</span></span>
      ${hubView.why ? `<span class="wk-line">${esc(hubView.why)}</span>` : ''}
    </span>${hubView.startable ? `<span class="wk-acts"><button type="button" class="ov-act" data-ov-hub-start${others.starting ? ' aria-disabled="true"' : ''}>${others.starting ? '起動中…' : 'hub を起動'}</button></span>` : ''}
  </div>`;
}

/* The sync button: its words are the time of the last sync; while one is out it says so and does nothing (aria-disabled, so the
   keyboard stays where it is). */
function othersSyncButtonHtml(lastSync, nowEst, stale, label = '', lite = false) {
  const at = stampSecs(lastSync?.at);
  const words = others.syncing ? '同期中…' : label || (at == null ? 'まだ同期していません' : `同期 · ${agoLabel(minutesSince(at, nowEst))}`);
  const tip = at == null ? '' : `最後の同期: ${when(lastSync.at)}`;
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
  const repo = othersEffectiveRepo(records, prefs.othersRepo);
  const marks = othersMarks();
  const hubView = othersHubView(doc.hub, records);
  const never = !lastSync?.at;

  const chips = records.length ? [{ key: '', label: 'すべて' }, ...othersRepoChips(records)] : [];
  const chipsHtml = chips.length
    ? `<div class="ov-chips" role="group" aria-label="リポジトリで絞り込む">${chips.map(c => `<button type="button" class="ov-chip" data-ov-repo="${esc(c.key)}" aria-pressed="${c.key === repo}">${esc(c.label)}</button>`).join('')}</div>` : '';
  const alerts = [
    ...(others.error ? [{ alert: true, text: '最新の状態を読めませんでした（表示は前回のものです）' }] : []),
    ...othersNotes(lastSync),
  ].map(n => n.alert ? `<div class="wk-notice" role="alert">${esc(n.text)}</div>` : `<div class="ov-note">${esc(n.text)}</div>`).join('');
  const staleHtml = stale
    ? `<div class="ov-stale" role="status">最後に同期してから ${othersStaleMinutes(lastSync, nowEst)} 分たっています ${othersSyncButtonHtml(lastSync, nowEst, false, 'いま同期する', true)}</div>` : '';

  let body;
  if (never && !records.length) {
    body = `<div class="wk-empty">まだ同期していません。「同期」で、あなたにレビューを頼んでいる PR を読み込みます<br>${othersSyncButtonHtml(lastSync, nowEst, false, 'いま同期する', true)}</div>`;
  } else if (!records.length) {
    body = '<div class="wk-empty">あなたにレビューを頼んでいる PR はありません</div>';
  } else {
    body = othersGroups(records, repo).map(g => `<section class="wk-group" aria-label="${esc(g.label)}">
      <div class="wk-head repo"><span class="wk-head-main"><span class="wk-head-name">${esc(g.label)}${g.unregistered ? ' <span class="ov-unreg" title="adj に登録していないリポジトリです（同じオーナーなので依頼だけ出ます）">未登録</span>' : ''}</span></span><span class="wk-count">${g.rows.length}</span></div>
      <div class="wk-body">${g.rows.map(r => othersRowHtml(r, othersSeenClass(r, marks[r.id] || {}) === 'new', hubView, nowEst, records)).join('')}</div>
    </section>`).join('');
  }
  return `<div class="ov-sync-row">${othersSyncButtonHtml(lastSync, nowEst, stale)}${chipsHtml}</div>${alerts}${staleHtml}${othersHubHtml(hubView)}${body}`;
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
  if (d.ovHubStart != null) return ['[data-ov-hub-start]', '[data-ov-hub]'];
  if (d.ovSplit != null) return [q('ov-split', d.ovSplit), rowOf(d.ovSplit)];
  if (d.ovRepo != null) return [q('ov-repo', d.ovRepo)];
  if (d.ovSync != null) return [el.closest('.ov-stale') ? '.ov-stale [data-ov-sync]' : '.ov-sync-row [data-ov-sync]', '[data-ov-sync]'];
  return [];
}

function renderOthersList(force = false) {
  const root = wk('ov-groups');
  if (!root || prefs.workTab !== 'others') return;
  const sig = JSON.stringify([others.doc ? othersSig(others.doc, othersNowEst()) : null, prefs.othersRepo, othersMarks(), others.error, others.syncing, others.syncError, [...others.asking], others.starting, nav.task]);
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
  // 分割案 is a later issue's, and the hub's row opens nothing.
  if (e.target.closest('[data-ov-split], [data-ov-hub]')) return;
  if ((b = e.target.closest('.ov-row[data-ov]'))) {
    const rec = othersById(b.dataset.ov);
    if (rec) othersSelect(rec);
  }
});
