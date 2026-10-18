/* ── The organisation switch at the head of the sidebar ────────────────────────────────────────────
   One button (the organisation's avatar, with how many things wait in the others) and a menu of the organisations. What it
   selects is `currentOrg()` (org.js); the list in 「いまの仕事」 and the request dialog read that. */

/* Not `wk` (my-work.js): this file is read before it, and binds its listeners as it loads. */
const orgEl = id => document.getElementById(id);
const orgMenuItems = () => [...orgEl('org-menu').querySelectorAll('[role="menuitemradio"]')];
const orgInitial = name => ([...String(name || '')][0] || '?').toUpperCase();
/* Avatars that failed to load: not asked for again when the switch is drawn again. */
const orgBadAvatars = new Set();

/* The avatar: the initial, with the picture over it when GitHub gave one. A picture that fails is taken off (`orgWatchAvatars`),
   which leaves the initial. */
function orgAvatarHtml(org) {
  const url = orgIcon(org.login)?.avatarUrl;
  const img = url && /^https:\/\//.test(url) && !orgBadAvatars.has(url)
    ? `<img class="org-ava" src="${esc(url)}" referrerpolicy="no-referrer" alt="" data-org-img>` : '';
  return `<span class="org-ava-box"><span class="org-ini" aria-hidden="true">${esc(orgInitial(org.name))}</span>${img}</span>`;
}

/* The `error` listener of each picture, attached after the markup is in (no inline handler); one that failed before the listener was there is found by `complete`. */
function orgWatchAvatars(root) {
  for (const img of root.querySelectorAll('img[data-org-img]')) {
    const drop = () => { orgBadAvatars.add(img.getAttribute('src')); img.remove(); };
    img.addEventListener('error', drop, { once: true });
    if (img.complete && !img.naturalWidth) drop();
  }
}

function renderOrgSwitch() {
  const btn = orgEl('org-btn');
  if (!btn) return;
  const menu = orgEl('org-menu');
  const orgs = multiBoard ? orgList(boards, work.doc) : [];
  if (!orgs.length) {
    // Nothing to choose between (one board served alone, or nothing registered yet): the plain badge.
    closeOrgMenu();
    btn.hidden = true;
    orgEl('org-fallback').hidden = false;
    orgEl('org-name').textContent = 'ボード';
    return;
  }
  const current = currentOrg();
  const now = orgs.find(o => o.login === current) || orgs[0];
  // What the TODO would list after switching, by organisation: the TODO's own gathering, not narrowed.
  const counts = work.doc ? orgWaitCounts(todoMine(() => true)) : new Map();
  const others = [...counts].filter(([login]) => login !== now.login).reduce((n, [, v]) => n + v, 0);
  orgEl('org-fallback').hidden = true;
  btn.hidden = false;
  orgEl('org-name').textContent = now.name;
  const sig = JSON.stringify([now.login, others, orgs.map(o => [o.login, o.name, orgIcon(o.login), counts.get(o.login) || 0]), [...orgBadAvatars]]);
  if (btn.dataset.sig === sig) return;
  btn.dataset.sig = sig;
  const label = `組織: ${now.name}（ほかの組織で待っているもの ${others} 件）`;
  btn.setAttribute('aria-label', label);
  btn.title = label;
  btn.innerHTML = `${orgAvatarHtml(now)}<span class="rail-badge org-badge${others ? '' : ' zero'}">${others}</span>`;
  // The menu keeps the keyboard focus through a redraw while it is open.
  const held = menu.contains(document.activeElement) ? document.activeElement.dataset.org : null;
  menu.innerHTML = orgs.map(o => {
    const n = counts.get(o.login) || 0;
    const kind = orgIcon(o.login)?.kind === 'user' ? '<span class="org-tag">個人</span>' : '';
    return `<button type="button" class="org-item" role="menuitemradio" aria-checked="${o.login === now.login}" data-org="${esc(o.login)}" tabindex="-1">${orgAvatarHtml(o)}<span class="org-item-name">${esc(o.name)}${kind}</span><span class="org-item-count${n ? '' : ' zero'}" title="${esc(`待っているもの ${n} 件`)}">${n}</span></button>`;
  }).join('');
  orgWatchAvatars(btn);
  orgWatchAvatars(menu);
  if (held != null) orgMenuItems().find(b => b.dataset.org === held)?.focus({ preventScroll: true });
}

function openOrgMenu() {
  orgEl('org-menu').hidden = false;
  orgEl('org-btn').setAttribute('aria-expanded', 'true');
  (orgMenuItems().find(b => b.getAttribute('aria-checked') === 'true') || orgMenuItems()[0])?.focus();
}

/* Shut. `refocus` gives the keyboard back to the button, for when the menu was used from the keyboard or chosen from. */
function closeOrgMenu(refocus = false) {
  const menu = orgEl('org-menu');
  if (!menu || menu.hidden) return;
  menu.hidden = true;
  orgEl('org-btn').setAttribute('aria-expanded', 'false');
  if (refocus) orgEl('org-btn').focus();
}

/* Choose an organisation. A selection in another one is cleared, as nothing of it is in the list any more. */
function chooseOrg(login) {
  closeOrgMenu(true);
  if (login === currentOrg()) return;
  setCurrentOrg(login);
  const sel = workSelected();
  const nwo = (sel?.repo || sel?.parent?.repo)?.nwo;
  if (nwo && orgOf(nwo) !== currentOrg()) go({ board: 'all', view: 'work', task: null, pane: 'detail' });
}

orgEl('org-btn').addEventListener('click', () => { if (orgEl('org-menu').hidden) openOrgMenu(); else closeOrgMenu(true); });
orgEl('org-menu').addEventListener('click', e => {
  const b = e.target.closest('[data-org]');
  if (b) chooseOrg(b.dataset.org);
});
orgEl('org-menu').addEventListener('keydown', e => {
  const items = orgMenuItems();
  const at = items.indexOf(document.activeElement);
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeOrgMenu(true); }
  else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    items[(at + (e.key === 'ArrowDown' ? 1 : items.length - 1) + items.length) % items.length].focus();
  } else if (e.key === 'Tab') closeOrgMenu();
});
document.addEventListener('pointerdown', e => {
  if (!e.target.closest('#org-menu, #org-btn')) closeOrgMenu();
});

// Another organisation was selected (the menu, or the address moved to a row of one): the filter on one repository does not carry over.
onOrgChange(() => {
  prefs.workRepo = '';
  savePrefs();
  renderOrgSwitch();
  if (view === 'work') renderWorkList();
});
