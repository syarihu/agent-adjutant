// Clip: search and jump to a session with ⌘K.
import { caption, press, sleep, title, visible, byText } from '../lib.mjs';

export const name = 'search-palette';

export async function play(rec) {
  const { page } = rec;
  await title(rec, {
    ja: 'セッション検索<small>⌘K で探して、そのまま開く</small>',
    en: 'Session search<small>Find a session with ⌘K and open it</small>',
  });

  await caption(rec, {
    ja: '⌘K を押して検索パレットを開く',
    en: 'Press ⌘K to open the search palette',
  });
  await press(rec, 'Meta+K', '⌘ K');
  await visible(page, '#palette').waitFor({ timeout: 10000 });
  await sleep(600);

  await caption(rec, {
    ja: 'あなたの返事を待っているセッションが先頭に並ぶ',
    en: 'Sessions waiting for your answer come first',
  });
  await sleep(2200);

  await caption(rec, {
    ja: 'タイトル・キー・ブランチ・リポジトリで絞り込める',
    en: 'Narrow by title, key, branch, or repository',
  });
  await page.keyboard.type('価格', { delay: 160 });
  await sleep(2200);

  await press(rec, 'Enter', 'Enter');
  await caption(rec, {
    ja: 'Enter で、選んだセッションが開く',
    en: 'Enter opens the chosen session',
  });
  // The chosen session opens in place of the palette, with its terminal.
  await page.locator('#palette').waitFor({ state: 'hidden', timeout: 10000 });
  await sleep(4000);

  await caption(rec, '');
  await sleep(800);
}
