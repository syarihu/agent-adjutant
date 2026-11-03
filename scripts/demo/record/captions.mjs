// node captions.mjs <frames dir>: draw each caption of the clip as a band image and write
// captions.txt, an ffmpeg concat list that lasts as long as the video.
import { chromium } from 'playwright-core';
import fs from 'node:fs';
import path from 'node:path';
import { W } from './lib.mjs';

export const BAND = 64;
const dir = process.argv[2];
const { duration, captions } = JSON.parse(fs.readFileSync(path.join(dir, 'captions.json'), 'utf8'));
const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: W, height: BAND }, deviceScaleFactor: 2 });
const draw = async (text, file) => {
  await page.setContent(`<html><body style="margin:0;height:${BAND}px;display:flex;align-items:center;justify-content:center;
    background:#111827;color:#f9fafb;font:600 21px/1.3 -apple-system,BlinkMacSystemFont,'Hiragino Sans','Segoe UI',sans-serif">
    <div>${text}</div></body></html>`);
  await page.screenshot({ path: file });
};
const list = [];
let at = 0, n = 0;
const push = async (text, len) => {
  if (len <= 0.01) return;
  const file = `cap${String(n++).padStart(3, '0')}.png`;
  await draw(text, path.join(dir, file));
  list.push(`file '${file}'\nduration ${len.toFixed(4)}`);
};
for (const c of captions) {
  await push('', c.from - at);
  await push(c.text, c.to - c.from);
  at = c.to;
}
await push('', duration - at);
list.push(list.at(-1).split('\n')[0]);
fs.writeFileSync(path.join(dir, 'captions.txt'), list.join('\n') + '\n');
await browser.close();
