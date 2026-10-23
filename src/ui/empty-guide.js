/* ── The first-run guide: what the page says while the resident server knows no repository ──────
   Right after install `/api/boards` answers `[]`, and the two screens that list work would show empty
   columns with nothing saying that `adj hub` run inside a repository is what adds one. The guide takes
   the main area over then, and goes by itself when the first board shows up in the poll. It carries the
   machine checks of server-checks.js (the banner steps aside meanwhile) as a checklist, so that what is
   missing and the one command that follows are in one place. The server's strings reach the page only
   through `esc`. */
const EG_HUB_CMD = 'adj hub';
const EG_LABELS = {
  config: ['設定ファイルを直す', '設定ファイルを読めました'],
  tmux: ['tmux をインストールする', 'tmux があります'],
  'tmux-version': ['tmux を 3.1 以降にする', 'tmux のバージョンは足りています'],
  gh: ['gh をインストールする', 'gh があります'],
  'gh-auth': ['gh にログインする', 'gh にログイン済みです'],
  mcp: ['MCP サーバーを登録する', 'MCP サーバーを登録済みです'],
};

/* Whether the guide shows. `state` is `{multiBoard, boardsLoaded, boardCount, view, list, task}`. No answer yet
   (or a failed one) is not "no boards", and レビュー and TODO are left as they are: レビュー needs no repository. */
function egShows(state) {
  if (!state || !state.multiBoard || !state.boardsLoaded || state.boardCount !== 0) return false;
  return state.view === 'agent' || (state.view === 'work' && state.list === 'task' && !state.task);
}

/* The steps: every check in the server's order while any of them fails, then the command that is always the last. */
function egSteps(data) {
  const checks = scAnswered(data) ? data.checks.filter(c => c && typeof c === 'object') : [];
  const steps = checks.some(c => c.ok === false)
    ? checks.map(c => {
      const labels = EG_LABELS[c.id];
      const ok = c.ok !== false;
      return {
        kind: 'check',
        id: c.id,
        done: ok,
        fatal: !ok && !!c.fatal,
        label: labels ? labels[ok ? 1 : 0] : (c.message || String(c.id)),
        message: labels && c.message ? c.message : '',
        fix: ok ? '' : (c.fix || ''),
      };
    })
    : [];
  steps.push({ kind: 'hub', label: 'リポジトリで hub を起動する', fix: EG_HUB_CMD });
  return steps;
}

const egCmdHtml = cmd => `<div class="eg-cmd"><code class="eg-code">${esc(cmd)}</code><button type="button" class="eg-copy" data-eg-copy="${esc(cmd)}" aria-label="コピー" title="コピー"><span class="material-symbols-outlined" aria-hidden="true">content_copy</span></button></div>`;

function egStepHtml(s) {
  const cmd = s.fix ? egCmdHtml(s.fix) : '';
  if (s.kind !== 'check') return `<li class="eg-step eg-check eg-hub"><span class="material-symbols-outlined eg-icon" aria-hidden="true">terminal</span><span class="eg-body"><span class="eg-label">${esc(s.label)}</span>${cmd}</span></li>`;
  const cls = s.done ? 'eg-done' : s.fatal ? 'eg-todo eg-error' : 'eg-todo';
  const icon = s.done ? 'check_circle' : 'radio_button_unchecked';
  const state = s.done ? '済み' : 'やること';
  const msg = s.message ? `<span class="eg-msg">${esc(s.message)}</span>` : '';
  return `<li class="eg-step eg-check ${cls}" data-check="${esc(s.id)}"><span class="material-symbols-outlined eg-icon" aria-hidden="true">${icon}</span><span class="eg-body"><span class="eg-head"><span class="eg-label">${esc(s.label)}</span><span class="eg-state">${state}</span></span>${msg}${cmd}</span></li>`;
}

function egHtml(steps) {
  const recheck = steps.some(s => s.kind === 'check') ? '<div class="eg-actions"><button type="button" class="btn-m3-tonal eg-recheck" data-eg-recheck>再確認</button><span class="eg-recheck-note" role="status"></span></div>' : '';
  return '<h2 id="eg-title">リポジトリがまだありません</h2>'
    + '<p>リポジトリのディレクトリで <code>adj hub</code> を起動すると、そのリポジトリがここに出ます。</p>'
    + `<ol class="eg-steps">${steps.map(egStepHtml).join('')}</ol>`
    + recheck
    + '<p class="eg-note">hub が起動すると、このページが自動で読み込みます。</p>'
    + '<p class="eg-other">PR のレビュー依頼はリポジトリがなくても <button type="button" class="eg-link" data-action="review">レビュー</button> で見られます。</p>';
}

/* Read from the page's globals on each call, which is cheap: scDraw asks it on every poll of the checks. */
function egShown() {
  return egShows({ multiBoard, boardsLoaded, boardCount: boards.length, view: nav.view, list: nav.list, task: nav.task });
}

let egRendered = null;

function egDraw() {
  const box = document.getElementById('empty-guide');
  if (!box) return;
  const on = egShown();
  box.hidden = !on;
  // Only the body class hides the views: their inline display belongs to setView.
  document.body.classList.toggle('empty-guide-on', on);
  if (!on) return;
  // Written only when it changes: the page polls, and a rewrite would drop the focus from a copy button.
  const html = egHtml(egSteps(scData));
  if (html !== egRendered) { egRendered = html; box.innerHTML = html; }
}

function egSelect(code) {
  const sel = window.getSelection && window.getSelection();
  if (!sel || !code) return;
  const range = document.createRange();
  range.selectNodeContents(code);
  sel.removeAllRanges();
  sel.addRange(range);
}

async function egCopy(button) {
  const text = button.dataset.egCopy || '';
  try {
    if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('no clipboard');
    await navigator.clipboard.writeText(text);
  } catch {
    // No clipboard here (a plain-http origin, say): the text is selected so that it can be copied by hand.
    egSelect(button.parentElement && button.parentElement.querySelector('.eg-code'));
    return;
  }
  button.setAttribute('aria-label', 'コピーしました');
  button.title = 'コピーしました';
  setTimeout(() => { button.setAttribute('aria-label', 'コピー'); button.title = 'コピー'; }, 1500);
}

async function egRecheck(button) {
  const note = button.parentElement.querySelector('.eg-recheck-note');
  button.disabled = true;
  let r;
  try { r = await scRunCheck(); } finally { button.disabled = false; }
  // The guide may have been redrawn by the answer (scDraw), so the note is looked up again, and the focus that was on
  // the button that went with the old content is put on the new one.
  const fresh = document.querySelector('#empty-guide [data-eg-recheck]');
  if (fresh && fresh !== button && !button.isConnected) fresh.focus();
  const live = document.querySelector('#empty-guide .eg-recheck-note') || note;
  if (live) live.textContent = r === 'ok' ? '' : r === 'pending' ? 'まだ確認中です' : '再確認できませんでした';
}

if (typeof document !== 'undefined' && document.getElementById('empty-guide')) {
  document.getElementById('empty-guide').addEventListener('click', e => {
    const copy = e.target.closest('[data-eg-copy]');
    if (copy) { egCopy(copy); return; }
    const re = e.target.closest('[data-eg-recheck]');
    if (re) egRecheck(re);
  });
}
