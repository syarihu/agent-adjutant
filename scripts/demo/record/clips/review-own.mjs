// Clip: an AI read-through of the PR your own worker opened, and its findings handed back to the worker.
import { caption, click, moveTo, sleep, title, visible, byText } from '../lib.mjs';

export const name = 'review-own';

export async function play(rec) {
  const { page } = rec;
  await click(rec, byText(page, '在庫の確認を 1 回の問い合わせにまとめる'), { pause: 1200 });
  await title(rec, { ja: 'AI 下読み（エージェントが書いたコード）<small>worker が出した PR を、別の worker が読んでから渡す</small>',
                     en: 'AI read-through: code your agent wrote<small>Another worker reads the PR before it reaches you</small>' });
  await caption(rec, { ja: 'worker が PR を出したら、見出しの「下読みする」', en: 'When the worker opens a PR, press 「下読みする」 in the heading' });
  await click(rec, visible(page, 'button:has-text("下読みする")'), { pause: 1000 });
  await caption(rec, { ja: '読むだけの worker が差分を読み、解説と気になるところを書く', en: 'A read-only worker reads the diff and writes notes and concerns' });
  // The read-through opens in a dialog of its own once the reader hands it in.
  const handOver = visible(page, 'button:has-text("worker に渡す")');
  await handOver.waitFor({ timeout: 60000 });
  await sleep(1800);
  await caption(rec, { ja: '要点と、差分の行ごとの解説', en: 'The point of the change, and notes on its lines' });
  await moveTo(rec, { x: 640, y: 420 });
  await page.mouse.wheel(0, 420); await sleep(2000);
  const concern = visible(page, 'text=50 件を超える SKU');
  await concern.scrollIntoViewIfNeeded(); await page.mouse.wheel(0, -160); await sleep(2200);
  await caption(rec, { ja: '指摘は下書きごと worker に渡せる', en: 'Hand the concerns, drafts and all, back to the worker' });
  await click(rec, visible(page, 'button:has-text("コメントにする")'), { pause: 1200 });
  await click(rec, visible(page, 'button:has-text("コメントを足す")'), { pause: 1200 });
  await click(rec, visible(page, 'button:has-text("worker に渡す")'), { pause: 1500 });
  await click(rec, page.locator('button:has-text("件を worker に渡す"), button:has-text("worker に渡す")').filter({ visible: true }).last(), { pause: 2000 });
  await page.keyboard.press('Escape'); await sleep(600);
  await page.keyboard.press('Escape'); await sleep(1500);
  await caption(rec, { ja: 'worker は指摘を受け取って直しに入る', en: 'The worker picks the findings up and starts fixing' });
  await sleep(5000);
  await caption(rec, '');
  await sleep(800);
}
