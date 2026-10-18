/* ── 依頼する: asking a repository's hub for a task, as one layer of the dialog stack ─────────────────────────
   The person says where (a repository of the organisation, and which of its hubs) and what; the request goes the way the new-task
   form sent it (`POST /api/tasks` on the hub's board, needing reading), so the hub reads it and asks before it starts anything. In a
   page with more than one board this is the one place to ask; a board served alone keeps its own form. Loaded after dialog-stack.js. */

/* The hubs a request to `repo` (nwo, case ignored) can go to, as { slug, label }: the repository's own first, then the parent tasks'
   by key. Only boards that are not finished take one. */
function requestHubOptions(boardList, repo) {
  const mine = (boardList || []).filter(b => !b.finished && String(b.nwo || '').toLowerCase() === String(repo || '').toLowerCase());
  const own = mine.filter(b => !b.hub).map(b => ({ slug: b.slug, label: 'リポジトリ hub' }));
  const parents = mine.filter(b => b.hub).sort((a, b) => (String(a.hub) < String(b.hub) ? -1 : String(a.hub) > String(b.hub) ? 1 : 0))
    .map(b => ({ slug: b.slug, label: `親 hub · ${[b.hub, b.title].filter(Boolean).join(' ')}` }));
  return [...own, ...parents];
}

/* The repositories of the organisation `login` that a request can go to (they have a board that is not finished), as nwo. */
const requestRepos = (orgs, boardList, login) => ((orgs || []).find(o => o.login === login)?.repos || []).filter(r => requestHubOptions(boardList, r).length);

/* The board the dialog opens on: the one named, else the one the address is on while it is not finished (and, in the work view,
   while it is in the organisation shown), else the own board of the repository the list is narrowed to, else the first repository's own. */
function requestPickBoard(boardList, { slug = null, navBoard = null, inWork = false, org = null, workRepo = '' } = {}) {
  const live = (boardList || []).filter(b => !b.finished);
  const ownerOf = b => String(b.nwo || '').split('/').length > 1 ? String(b.nwo).split('/')[0].toLowerCase() : '';
  const named = live.find(b => b.slug === slug);
  if (named) return named;
  const at = live.find(b => b.slug === navBoard);
  if (at && (!inWork || org == null || ownerOf(at) === org)) return at;
  const inOrg = live.filter(b => org == null || ownerOf(b) === org);
  const own = repo => inOrg.find(b => !b.hub && String(b.nwo).toLowerCase() === repo);
  return (workRepo && own(workRepo)) || inOrg.find(b => !b.hub) || inOrg[0] || null;
}

/* The body of the request: only the person's words (the hub reads what is in them), needing reading, with the parent when the
   dialog was opened for a child. */
const requestBody = (text, status, parent = null) => ({ body: text, status, needsReading: true, ...(parent ? { parent } : {}) });

/* Ctrl or ⌘ with Enter sends the request, unless the Enter is the IME's that confirms a conversion. */
const requestSendKey = e => e.key === 'Enter' && !!(e.ctrlKey || e.metaKey) && !e.isComposing && e.keyCode !== 229;

let requestBusy = false;
/* What was typed and not sent, kept between openings. */
let requestDraft = '';
let requestParent = null;

const requestOptionsHtml = (options, chosen) => options.map(o => `<option value="${esc(o.value)}"${o.value === chosen ? ' selected' : ''}>${esc(o.label)}</option>`).join('');

/* Open the dialog. `slug` is the board to ask (a child form's parent is on one); `parent` is the parent task's URL. */
function openRequestDialog({ parent = null, slug = null } = {}) {
  if (!multiBoard || dialogStack.has('request')) return;
  const live = boards.filter(b => !b.finished);
  if (!live.length) return note('依頼する', true, '依頼できるボードがありません');
  const here = currentOrg();
  const first = requestPickBoard(boards, { slug, navBoard: nav.board, inWork: view === 'work', org: here, workRepo: prefs.workRepo });
  const login = first ? orgOf(first.nwo) : here;
  const repos = requestRepos(orgList(boards, work.doc), boards, login);
  const repo = first && repos.find(r => r.toLowerCase() === String(first.nwo).toLowerCase()) || repos[0] || '';
  const hubs = requestHubOptions(boards, repo);
  const hub = hubs.find(h => h.slug === first?.slug)?.slug || hubs[0]?.slug || '';
  requestParent = parent;
  const body = `<div class="req-form">
    <label class="req-field"><span>リポジトリ</span><select data-req-repo>${requestOptionsHtml(repos.map(r => ({ value: r, label: r })), repo)}</select></label>
    <label class="req-field"><span>hub</span><select data-req-hub>${requestOptionsHtml(hubs.map(h => ({ value: h.slug, label: h.label })), hub)}</select></label>
    ${parent ? `<p class="req-parent">親タスク: ${esc(parent)}</p>` : ''}
    <textarea class="req-input" data-req-text placeholder="やってほしいこと" aria-label="依頼の内容">${esc(parent ? '' : requestDraft)}</textarea>
    <p class="req-error" role="alert" data-req-error></p>
  </div>`;
  const foot = `<div class="req-foot"><span class="req-hint">hub が読んで、着手前にあなたに確認します</span>
    <button type="button" class="btn-m3-tonal" data-req-send="backlog">あとで（Backlog）</button>
    <button type="button" class="btn-m3-primary" data-req-send="queued">渡す</button></div>`;
  dialogStack.push({
    key: 'request', title: '依頼する', body, foot, opener: document.activeElement,
    onClose: layer => {
      // An unsent text stays for the next opening; a child's is not kept, as its parent is not.
      const typed = dialogStack.layerEl(layer.key)?.querySelector('[data-req-text]')?.value;
      if (typed != null && !requestParent) requestDraft = typed;
      requestParent = null;
    },
  });
  dialogStack.layerEl('request')?.querySelector('[data-req-text]')?.focus();
}

/* Send: refused with a line in the dialog when it is empty; otherwise one request at a time, both buttons dimmed meanwhile. */
async function requestSend(status) {
  const layer = dialogStack.layerEl('request');
  if (!layer || requestBusy) return;
  const error = layer.querySelector('[data-req-error]');
  const text = layer.querySelector('[data-req-text]').value.trim();
  const slug = layer.querySelector('[data-req-hub]').value;
  if (!text) { error.textContent = '依頼の内容を書いてください'; return; }
  if (!slug) { error.textContent = '依頼先の hub がありません'; return; }
  error.textContent = '';
  requestBusy = true;
  const buttons = [...layer.querySelectorAll('[data-req-send]')];
  for (const b of buttons) b.disabled = true;
  const line = 'adj task add --needs-reading --body -' + (status === 'queued' ? ' --queue' : '');
  try {
    const data = await boardApi(`/b/${slug}`, '/api/tasks', { method: 'POST', body: JSON.stringify(requestBody(text, status, requestParent)) });
    note(line, false, (status === 'queued' ? '記録して受信箱へ。hub が読んで、着手前にあなたに確認します' : 'Backlog は受信箱へ送信しません') + handedNote(data.handed));
    dialogStack.closeAll();
    // After the layer's own close, which would have kept the text.
    requestDraft = '';
    refreshWork(true);
    refresh(true);
  } catch (e) {
    // The text stays, so it can be sent again.
    error.textContent = e.message;
    note(`${line} → ${e.message}`, true);
  } finally {
    requestBusy = false;
    for (const b of buttons) b.disabled = false;
  }
}

/* The repository was changed: the hubs of that one. */
function requestRepoChanged(layer) {
  const hubs = requestHubOptions(boards, layer.querySelector('[data-req-repo]').value);
  layer.querySelector('[data-req-hub]').innerHTML = requestOptionsHtml(hubs.map(h => ({ value: h.slug, label: h.label })), hubs[0]?.slug);
}

const requestEl = e => (dialogStack.has('request') ? e.target.closest('[data-qd-key="request"]') : null);
document.getElementById('question-dialog').addEventListener('click', e => {
  const b = requestEl(e) && e.target.closest('[data-req-send]');
  if (b) requestSend(b.dataset.reqSend);
});
document.getElementById('question-dialog').addEventListener('change', e => {
  const layer = requestEl(e);
  if (layer && e.target.matches('[data-req-repo]')) requestRepoChanged(layer);
});
document.getElementById('question-dialog').addEventListener('keydown', e => {
  if (requestEl(e) && e.target.matches('[data-req-text]') && requestSendKey(e)) { e.preventDefault(); requestSend('queued'); }
});
