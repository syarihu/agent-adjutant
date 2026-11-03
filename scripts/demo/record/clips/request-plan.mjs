// Clip: ask the hub for a task, confirm how it read the request, and approve the worker's plan.
import { caption, click, moveTo, sleep, title, typeInto, visible, byText } from '../lib.mjs';

export const name = 'request-plan';

export async function play(rec) {
  const { page } = rec;
  await click(rec, byText(page, 'リポジトリ hub'), { pause: 1200 });
  await title(rec, { ja: '依頼する → 計画の確認<small>やってほしいことを書くだけで、hub が読んで worker に渡す</small>',
                     en: 'Ask, then approve the plan<small>Write what you want; the hub reads it and hands it to a worker</small>' });

  await caption(rec, { ja: '「依頼する」に、やってほしいことをそのまま書く', en: 'Write what you want in 「依頼する」 (Ask)' });
  await click(rec, visible(page, '#wk-req-btn'), { pause: 700 });
  await typeInto(rec, visible(page, '[data-req-text]'), 'クーポンが有効期限の当日に使えないので直してほしい');
  await sleep(600);
  await click(rec, visible(page, '[data-req-send="queued"]'), { pause: 800 });

  await caption(rec, { ja: 'hub が依頼を読み、着手する前に読み取った内容を確認してくる', en: 'The hub reads it and checks its reading with you before starting' });
  const start = visible(page, 'button:has-text("着手する")');
  await start.waitFor({ timeout: 60000 });
  await sleep(1800);
  await moveTo(rec, start);
  await sleep(1200);
  await click(rec, start, { pause: 1000 });

  await caption(rec, { ja: 'worker が自分の worktree で調べ、計画を出す', en: 'A worker investigates in its own worktree and writes a plan' });
  await click(rec, byText(page, '有効期限の当日にクーポンが使えない'), { pause: 500 });
  const open = visible(page, '[data-qc-open-gate]');
  await open.waitFor({ timeout: 60000 });
  await sleep(1500);

  await caption(rec, { ja: '計画は質問カードで読む。決めることだけが問われる', en: 'The plan arrives as a question card that asks only what needs deciding' });
  await click(rec, open, { pause: 1500 });
  const dialog = visible(page, '#question-dialog');
  await page.mouse.wheel(0, 300); await sleep(1200);
  await page.mouse.wheel(0, 300); await sleep(1200);
  const choice = visible(page, '#question-dialog button:has-text("A.")');
  await moveTo(rec, choice); await sleep(1000);
  await caption(rec, { ja: 'おすすめの案を選ぶと、worker が実装に進む', en: 'Pick an option and the worker moves on to implementing' });
  await click(rec, choice, { pause: 1500 });
  await sleep(4000);
  await caption(rec, '');
  await sleep(800);
}
