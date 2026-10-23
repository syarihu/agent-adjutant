/* ── The banner at the top: what the resident found wrong on this machine ───────────────────
   `GET /api/server` lists the checks (tmux, gh and its login, the MCP registration, the config) with
   the words for each, as `adj server start` says them. The banner shows only the ones that failed, and
   goes away once every one passes. Closing it hides it for this tab only, until the set of failures
   changes. The server's strings reach the page only through `esc`. */
const SC_POLL_MS = 60000;
const SC_DISMISS_KEY = 'adj-server-checks-dismissed';
const SC_LABELS = {
  config: '設定ファイルを読めません',
  tmux: 'tmux が見つかりません',
  'tmux-version': 'tmux が古いです',
  gh: 'gh が見つかりません',
  'gh-auth': 'gh にログインしていません',
  mcp: 'MCP サーバーが登録されていません',
};

/* The checks of the answer that failed; an answer that is not the list (a board served alone has no such route) has none. */
const scFailing = data => (data && Array.isArray(data.checks) ? data.checks : []).filter(c => c && c.ok === false);

/* Whether an answer carries a result: before the resident's first check ends it says `checkedAt: null` and lists
   nothing, which is "not known yet", not "everything passes", so the banner is left as it was. */
const scAnswered = data => !!data && data.checkedAt != null && Array.isArray(data.checks);

/* What a dismissal is keyed by: the failing ids, sorted, so a new failure shows the banner again. */
const scKey = failing => failing.map(c => String(c.id)).sort().join(',');

/* Whether the banner shows: something failed, and it is not the very set that was closed in this tab. */
const scVisible = (failing, dismissedKey) => failing.length > 0 && scKey(failing) !== dismissedKey;

const scLabel = c => SC_LABELS[c.id] || c.message || String(c.id);

/* One row for each failure: the label, the server's words smaller, and the fix as code. Fatal ones are errors, the rest warnings. */
function scHtml(failing) {
  return failing.map(c => {
    const msg = c.message && scLabel(c) !== c.message ? `<span class="sc-msg">${esc(c.message)}</span>` : '';
    const fix = c.fix ? `<code class="sc-fix">${esc(c.fix)}</code>` : '';
    return `<div class="sc-item ${c.fatal ? 'sc-error' : 'sc-warn'}" data-check="${esc(c.id)}"><span class="sc-label">${esc(scLabel(c))}</span>${msg}${fix}</div>`;
  }).join('');
}

let scData = null;
let scShown = null;

function scDismissed() {
  try { return sessionStorage.getItem(SC_DISMISS_KEY); } catch { return null; }
}

function scDraw() {
  const box = document.getElementById('server-checks');
  if (!box) return;
  const failing = scFailing(scData);
  // Once everything passes, a closed set is forgotten: the same failure coming back later is news again.
  if (!failing.length) { try { sessionStorage.removeItem(SC_DISMISS_KEY); } catch { /* nothing to forget then */ } }
  // The first-run guide carries the same failures, so the banner steps aside while it shows.
  box.hidden = egShown() || !scVisible(failing, scDismissed());
  // Written only when it changes: the banner is a live region, and the same list written again on every poll would be
  // read out again by a screen reader.
  const html = scHtml(failing);
  if (html !== scShown) {
    scShown = html;
    document.getElementById('sc-list').innerHTML = html;
  }
  egDraw();
}

async function scLoad() {
  try {
    const data = await boardApi('', '/api/server');
    if (!scAnswered(data)) return;
    scData = data;
    document.getElementById('sc-note').textContent = '';
  } catch {
    // The banner stays as it was: no answer is not "everything passes".
    return;
  }
  scDraw();
}

/* Asks the resident to check again and keeps the answer; both the banner and the first-run guide use it. Returns
   'ok' when the answer carries a result, 'pending' when the first check has not ended, 'failed' otherwise. */
async function scRunCheck() {
  try {
    const data = await boardApi('', '/api/server/check', { method: 'POST', body: '{}' });
    if (!scAnswered(data)) return 'pending';
    scData = data;
    scDraw();
    return 'ok';
  } catch {
    return 'failed';
  }
}

async function scRecheck() {
  const button = document.getElementById('sc-recheck');
  const note = document.getElementById('sc-note');
  button.disabled = true;
  try {
    const r = await scRunCheck();
    note.textContent = r === 'ok' ? '' : r === 'pending' ? 'まだ確認中です' : '再確認できませんでした';
  } finally {
    button.disabled = false;
  }
}

function scClose() {
  try { sessionStorage.setItem(SC_DISMISS_KEY, scKey(scFailing(scData))); } catch { /* the banner is hidden for now all the same */ }
  scDraw();
}

if (typeof document !== 'undefined' && document.getElementById('server-checks')) {
  document.getElementById('sc-recheck').addEventListener('click', scRecheck);
  document.getElementById('sc-close').addEventListener('click', scClose);
  scLoad();
  setInterval(scLoad, SC_POLL_MS);
}
