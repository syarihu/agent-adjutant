// Clip: show every stage in the 工程 list view and switch between list and columns.
import { caption, click, moveTo, sleep, title, visible, byText } from '../lib.mjs';

export const name = 'boards-list';

export async function play(rec) {
  const { page } = rec;
  await title(rec, {
    ja: '工程<small>タスクがいまどこにあって、誰を待っているか</small>',
    en: 'Stages<small>Where each task is, and who it waits on</small>',
  });

  await caption(rec, {
    ja: '「工程」では、タスクがどこまで進んで誰を待っているかを一覧できる',
    en: 'Open 「工程」 (Stages) to see where every task stands',
  });
  await click(rec, visible(page, '#nav-boards'), { pause: 800 });
  await visible(page, '#board-list').waitFor({ timeout: 15000 });
  await sleep(1000);

  await caption(rec, {
    ja: 'いちばん上は、あなたの確認を待っているタスク',
    en: 'At the top: tasks waiting for you to check them',
  });
  await moveTo(rec, visible(page, '[data-col="me"] .stage-head'));
  await sleep(2000);

  await caption(rec, {
    ja: '承認済みで、あとはマージするだけの PR',
    en: 'Approved pull requests waiting only to be merged',
  });
  await moveTo(rec, visible(page, '[data-col="merge"] .stage-head'));
  await sleep(2000);

  await caption(rec, {
    ja: '他の人のレビューを待っている PR',
    en: 'Pull requests waiting on a review from someone else',
  });
  await moveTo(rec, visible(page, '[data-col="rev"] .stage-head'));
  await sleep(2000);

  await caption(rec, {
    ja: 'AI のレビュー中や、計画・実装中のタスクは worker が進めている',
    en: 'Tasks in AI review, planning or implementation are the workers\' to move',
  });
  await moveTo(rec, visible(page, '[data-col="ai"] .stage-head'));
  await sleep(1200);
  await moveTo(rec, visible(page, '[data-col="work"] .stage-head'));
  await sleep(2000);

  await page.mouse.wheel(0, 240);
  await sleep(600);

  await caption(rec, {
    ja: '着手前のキューと、完了したタスク',
    en: 'Queued tasks waiting to start, and completed tasks',
  });
  await moveTo(rec, visible(page, '[data-col="before"] .stage-head'));
  await sleep(1200);
  await moveTo(rec, visible(page, '[data-col="done"] .stage-head'));
  await sleep(2000);

  await page.mouse.wheel(0, -240);
  await sleep(600);

  await caption(rec, {
    ja: '「列」に切り替えると、同じ工程を列に並べて見られる',
    en: 'Switch to 「列」 (Columns) to see the stages side by side',
  });
  await click(rec, visible(page, '[data-board-view="cols"]'), { pause: 500 });
  await sleep(3000);

  await caption(rec, {
    ja: 'ふだんはリスト表示。工程を上から順に読める',
    en: 'The list is the default: the stages read from top to bottom',
  });
  await click(rec, visible(page, '[data-board-view="list"]'), { pause: 800 });
  await sleep(2500);

  await caption(rec, '');
  await sleep(800);
}
