/* The popover of a stack: pressing the chip 「N/M」 of a card or row lists the stack top first, down to the branch it is cut
   from, each step with the state of its PR and the stage it is in. Pressing a step goes to that card or row. The frame is the
   TODO popover's (todo.css); the line that joins the steps is CSS alone. */

/* Which chip it is open for, by task id and board (`data-stack`, `data-stack-slug`); null when closed. */
let stackPop = null;

const STACK_PR_ICON = { open: ['merge', 'st-open'], draft: ['edit_note', 'st-draft'], merged: ['merge', 'st-merged'], closed: ['cancel', 'st-draft'] };
const stackPopEl = () => document.getElementById('stack-pop');
const boardTasks = () => [...(state.tasks || []), ...(state.hubTasks || [])];
const taskByRef = (id, slug) => boardTasks().find(t => t.id === id && slugKeyOf(t) === slug);
const stackChipOf = ref => [...document.querySelectorAll('#boards .stack-chip')]
  .find(c => c.dataset.stack === ref.id && c.dataset.stackSlug === ref.slug) || null;

/* What names a task in the popover: its Issue's number or key, else its id. */
function taskKeyOf(task) {
  const url = httpUrl(task.issueUrl);
  return (url && issueLabelOf(url).replace(/^Issue\s*/, '')) || task.id;
}

/* The popover's heading, label and steps for `task`'s stack, or null when it is in none. The steps run from the top of the stack
   down; a step whose task this board does not list (a merged one, or another hub's) has no card to go to, so it is not a button. */
function stackPopParts(task) {
  const st = stackOf(task);
  if (!st) return null;
  const tasks = boardTasks();
  const parent = st.group.number ? `#${st.group.number}` : st.group.key;
  const steps = [...st.chain].reverse().map(c => {
    const x = taskOfChild(c, st.group, tasks);
    const prState = x ? prStateOf(x) : c.merged ? 'merged' : null;
    const [icon, cls] = STACK_PR_ICON[prState] || ['code', 'st-draft'];
    const pr = x && prRefNumber(x.pr) ? `#${prRefNumber(x.pr)}` : null;
    const stage = x ? agentLabel(agentColOf(x)) : c.merged ? '完了' : null;
    const sub = [pr, c.branch, stage].filter(Boolean).join(' · ');
    const title = `${x ? taskKeyOf(x) : ''} ${c.title || x?.title || `#${c.id}`}`.trim();
    const body = `<span class="st-title" title="${esc(title)}">${esc(title)}</span><span class="st-sub">${esc(sub)}</span>`;
    const here = c === st.chain[st.at];
    return `<li class="stack-step${here ? ' here' : ''}"${here ? ' aria-current="true"' : ''}><span class="material-symbols-outlined ${cls}" aria-hidden="true">${icon}</span>`
      + (x ? `<button type="button" class="stack-go" data-goto="${esc(x.id)}" data-goto-slug="${esc(slugKeyOf(x))}">${body}</button>` : `<span class="stack-go">${body}</span>`)
      + '</li>';
  }).join('');
  const base = st.root.base;
  return {
    label: `スタック・親 ${parent}`,
    html: `<div class="stack-pop-head"><span>Stack · 親 ${esc(parent)}</span><button type="button" class="m3-icon-button" data-pop-close aria-label="閉じる"><span class="material-symbols-outlined" aria-hidden="true" style="font-size:20px;">close</span></button></div>`
      + `<div class="todo-scroll"><ul class="stack-steps">${steps}</ul>`
      + `${base ? `<div class="stack-foot"><span class="material-symbols-outlined" aria-hidden="true">radio_button_unchecked</span><code>${esc(base)}</code></div>` : ''}</div>`,
  };
}

/* Where the chip was when the popover was placed beside it: a scroll that moves the chip leaves the popover behind. */
let stackPopAt = null;

/* Beside the chip, below it unless that runs off the window. */
function placeStackPop(chip) {
  const pop = stackPopEl();
  const r = chip.getBoundingClientRect(), w = pop.offsetWidth, h = pop.offsetHeight;
  stackPopAt = { left: r.left, top: r.top };
  pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  pop.style.top = `${r.bottom + 6 + h > innerHeight ? Math.max(8, r.top - 6 - h) : r.bottom + 6}px`;
}

function openStackPop(chip) {
  const task = taskByRef(chip.dataset.stack, chip.dataset.stackSlug);
  const parts = task && stackPopParts(task);
  if (!parts) return;
  closeStackPop();
  const pop = stackPopEl();
  pop.setAttribute('aria-label', parts.label);
  pop.innerHTML = parts.html;
  pop.hidden = false;
  stackPop = { id: chip.dataset.stack, slug: chip.dataset.stackSlug };
  chip.setAttribute('aria-expanded', 'true');
  placeStackPop(chip);
  (pop.querySelector('.stack-step.here .stack-go') || pop.querySelector('[data-pop-close]')).focus();
}

/* `refocus` returns the focus to the chip, as Esc and the close button do; a press elsewhere keeps it where it went. */
function closeStackPop(refocus = false) {
  const pop = stackPopEl();
  if (!pop || pop.hidden) return;
  pop.hidden = true;
  const chip = stackPop && stackChipOf(stackPop);
  stackPop = null;
  if (chip) {
    chip.setAttribute('aria-expanded', 'false');
    if (refocus) chip.focus();
  }
}

/* After the board is drawn again: the chip it was open for is a new element now. If its task is still in a stack the popover is
   drawn from what is known now, in the same place; else it closes. */
function syncStackPop() {
  const pop = stackPopEl();
  if (!stackPop || !pop || pop.hidden) return;
  const task = taskByRef(stackPop.id, stackPop.slug);
  const chip = stackChipOf(stackPop);
  const parts = task && chip && stackPopParts(task);
  if (!parts) return closeStackPop();
  const focused = pop.contains(document.activeElement) ? document.activeElement : null;
  const goto = focused?.dataset.goto, slug = focused?.dataset.gotoSlug;
  const top = pop.querySelector('.todo-scroll')?.scrollTop || 0;
  pop.setAttribute('aria-label', parts.label);
  pop.innerHTML = parts.html;
  chip.setAttribute('aria-expanded', 'true');
  pop.querySelector('.todo-scroll').scrollTop = top;
  placeStackPop(chip);
  if (focused) {
    const again = goto != null ? [...pop.querySelectorAll('[data-goto]')].find(b => b.dataset.goto === goto && b.dataset.gotoSlug === slug) : null;
    (again || pop.querySelector('[data-pop-close]')).focus({ preventScroll: true });
  }
}

/* Brings the card or row of a task into view and marks it with the flash the board uses for a jump. */
function goToBoardItem(id, slug) {
  let el = boardItemOf(id, slug);
  // A done card older than a day is folded behind 「以前の完了」: opened, so that there is a card to go to.
  if (!el && !showOlderDone) {
    showOlderDone = true;
    render();
    el = boardItemOf(id, slug);
  }
  if (!el) return;
  el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'smooth' });
  el.classList.remove('flash');
  void el.offsetWidth;
  el.classList.add('flash');
  setTimeout(() => el.classList.remove('flash'), 1700);
}

document.addEventListener('click', e => {
  const chip = e.target.closest('#boards .stack-chip');
  if (chip) {
    e.preventDefault();
    return stackPop && chip.getAttribute('aria-expanded') === 'true' ? closeStackPop(true) : openStackPop(chip);
  }
  const pop = stackPopEl();
  if (!pop || !pop.contains(e.target)) return;
  if (e.target.closest('[data-pop-close]')) return closeStackPop(true);
  const go = e.target.closest('[data-goto]');
  if (go) {
    closeStackPop();
    goToBoardItem(go.dataset.goto, go.dataset.gotoSlug);
  }
});
document.addEventListener('pointerdown', e => {
  const pop = stackPopEl();
  if (stackPop && pop && !pop.contains(e.target) && !e.target.closest('.stack-chip')) closeStackPop();
});
document.addEventListener('keydown', e => {
  if (e.key !== 'Escape' || e.isComposing || !stackPop) return;
  e.preventDefault();
  closeStackPop(true);
});
// The popover is fixed to the window, so it is closed where it would be left behind: a resize, a scroll that moved the chip (a
// redraw puts the scroll back to where it was and moves nothing), and focus leaving it for something else.
window.addEventListener('resize', () => closeStackPop());
function onStackScroll(e) {
  const pop = stackPopEl();
  if (!stackPop || !pop || pop.contains(e.target)) return;
  const chip = stackChipOf(stackPop);
  const r = chip?.getBoundingClientRect();
  if (!r || !stackPopAt || Math.abs(r.left - stackPopAt.left) > 1 || Math.abs(r.top - stackPopAt.top) > 1) closeStackPop();
}
// Focus going to the chip is the press that toggles the popover (a button takes focus on mousedown in some browsers): the click closes it.
function onStackFocusOut(e) {
  const to = e.relatedTarget;
  const pop = stackPopEl();
  if (stackPop && pop && to && !pop.contains(to) && !to.closest?.('.stack-chip')) closeStackPop();
}
document.addEventListener('scroll', onStackScroll, true);
stackPopEl()?.addEventListener('focusout', onStackFocusOut);
