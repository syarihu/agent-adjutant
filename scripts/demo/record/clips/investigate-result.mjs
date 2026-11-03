// Clip: a worker looks into a question, and its findings come back as a report to read.
import fs from 'node:fs';
import { caption, click, moveTo, sleep, title, visible, byText } from '../lib.mjs';

export const name = 'investigate-result';
const go = beat => fs.writeFileSync(`${process.env.DEMO_ROOT || '/tmp/adj-demo'}/go-${beat}`, '');

export async function play(rec) {
  const { page } = rec;
  await click(rec, byText(page, 'チェックアウトが遅い原因を調べる'), { pause: 1200 });
  await title(rec, { ja: '調査結果<small>調べるだけの依頼は、結論と根拠をそろえた報告で返ってくる</small>',
                     en: 'Investigation results<small>A question comes back as a report with its conclusion and evidence</small>' });
  await caption(rec, { ja: 'worker がコードを追い、ステージングで計測する', en: 'The worker follows the code and measures on staging' });
  go('checkout');
  const open = visible(page, '[data-qc-open-gate]');
  await open.waitFor({ timeout: 60000 });
  await sleep(1500);
  await caption(rec, { ja: '結果は「調査報告」としてボードに届く', en: 'The findings arrive on the board as a report' });
  await click(rec, open, { pause: 1800 });
  await caption(rec, { ja: '結論・原因・対応策が、最初に読む順に並ぶ', en: 'Conclusion, cause and fix, in the order you read them' });
  await page.mouse.wheel(0, 250); await sleep(1600);
  await page.mouse.wheel(0, 250); await sleep(1400);
  const report = visible(page, 'button:has-text("レポートを開く")');
  await caption(rec, { ja: '表やグラフは HTML レポートで添えられる', en: 'Tables and charts come as an HTML report' });
  await click(rec, report, { pause: 3500 });
  await page.keyboard.press('Escape'); await sleep(1000);
  const ack = visible(page, '#question-dialog button:has-text("分かった")');
  await caption(rec, { ja: '読んだら答えるだけ。直すなら、そのままタスクにできる', en: 'Answer when you have read it, or turn it into a task' });
  await click(rec, ack, { pause: 2500 });
  await caption(rec, '');
  await sleep(1000);
}
