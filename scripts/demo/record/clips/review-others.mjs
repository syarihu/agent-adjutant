// Clip: read someone else's pull request with an AI read-through, and put comments in a pending review.
import { caption, click, inColumn, moveTo, sleep, title, visible, byText } from '../lib.mjs';

export const name = 'review-others';

export async function play(rec) {
  const { page } = rec;
  await click(rec, visible(page, '#nav-review'), { pause: 1200 });
  await title(rec, { ja: 'AI 下読み（他の人の PR）<small>頼まれたレビューを、読むだけの worker が先に読んでおく</small>',
                     en: 'AI read-through: a PR you were asked to review<small>A read-only worker reads it before you do</small>' });
  await caption(rec, { ja: 'レビューを頼まれた PR が並ぶ', en: 'The pull requests you were asked to review' });
  await click(rec, byText(page, 'ほしい物リストの API を追加'), { pause: 1400 });
  await caption(rec, { ja: '「下読みする」で、読むだけの worker がこの PR を読む', en: '「下読みする」 starts a read-only worker on the PR' });
  await click(rec, await inColumn(page, 'button:has-text("下読みする")', 400, 780), { pause: 800 });
  await visible(page, 'button:has-text("レビューを送る")').waitFor({ timeout: 60000 });
  await sleep(1200);
  await caption(rec, { ja: '要点を3行で。何がこの PR の本質かも書いてある', en: 'The point of the PR in three lines, and what it turns on' });
  await click(rec, visible(page, '#rail-toggle'), { pause: 1200 });
  await moveTo(rec, { x: 700, y: 420 });
  await page.mouse.wheel(0, 380); await sleep(2000);
  await caption(rec, { ja: '差分の行に解説と訳。気になるところには下書きのコメントも', en: 'Notes and translations on the diff lines, with a draft comment for each concern' });
  const concern = visible(page, 'button:has-text("コメントにする")');
  await concern.scrollIntoViewIfNeeded(); await page.mouse.wheel(0, -120); await sleep(2200);
  await click(rec, concern, { pause: 1500 });
  await click(rec, visible(page, 'button:has-text("コメントを足す")'), { pause: 1500 });
  await caption(rec, { ja: '選んだコメントは、GitHub の pending review にまとめて積む', en: 'Your comments go into a pending review on GitHub in one step' });
  await click(rec, visible(page, 'button:has-text("レビューを送る")'), { pause: 1500 });
  const push = visible(page, 'button:has-text("pending review に積む")');
  await moveTo(rec, push); await sleep(1000);
  await click(rec, push, { pause: 2500 });
  await caption(rec, { ja: '承認や変更依頼を決めて送るのは、あなた', en: 'Approving or requesting changes stays with you' });
  await sleep(2500);
  await caption(rec, '');
  await sleep(800);
}
