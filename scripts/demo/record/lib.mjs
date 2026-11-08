// The recorder's tools: a browser on the demo board, a cursor and captions drawn over the page
// (a headless browser shows no pointer), and frames taken through the DevTools screencast at
// twice the CSS size so the text stays sharp once the video is scaled down.
import { chromium } from 'playwright-core';
import fs from 'node:fs';
import path from 'node:path';

export const W = 1280, H = 760;

const OVERLAY_CSS = `
#demo-layer { position: fixed; inset: 0; width: 100vw; height: 100vh; max-width: none; max-height: none; margin: 0; padding: 0;
  border: 0; background: transparent; overflow: visible; pointer-events: none; }
#demo-cursor { position: fixed; z-index: 2147483647; width: 22px; height: 22px; left: 0; top: 0; pointer-events: none;
  transition: transform var(--move, 700ms) cubic-bezier(.4,0,.2,1); transform: translate(640px, 380px); }
#demo-cursor svg { filter: drop-shadow(0 1px 2px rgba(0,0,0,.45)); }
.demo-ripple { position: fixed; z-index: 2147483646; width: 36px; height: 36px; margin: -18px 0 0 -18px; border-radius: 50%;
  background: rgba(25,118,210,.35); pointer-events: none; animation: demo-ripple 520ms ease-out forwards; }
@keyframes demo-ripple { from { transform: scale(.3); opacity: 1; } to { transform: scale(1.6); opacity: 0; } }
#demo-caption { position: fixed; z-index: 2147483645; left: 50%; bottom: 28px; transform: translateX(-50%);
  max-width: 80%; padding: 12px 22px; border-radius: 14px; background: rgba(20,24,33,.88); color: #fff;
  font: 600 20px/1.45 -apple-system, BlinkMacSystemFont, "Hiragino Sans", "Segoe UI", sans-serif; text-align: center;
  pointer-events: none; opacity: 0; transition: opacity 260ms; box-shadow: 0 6px 24px rgba(0,0,0,.25); }
#demo-caption.on { opacity: 1; }
#demo-caption small { display: block; font-weight: 400; font-size: 15px; opacity: .8; }
#demo-title { position: fixed; inset: 0; z-index: 2147483647; display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 10px; background: #0f172a; color: #fff; pointer-events: none; transition: opacity 400ms;
  font: 700 40px/1.3 -apple-system, BlinkMacSystemFont, "Hiragino Sans", "Segoe UI", sans-serif; }
#demo-title small { font-weight: 400; font-size: 20px; opacity: .75; }
#demo-title.off { opacity: 0; }
#demo-key { position: fixed; z-index: 2147483647; left: 50%; bottom: 40px; transform: translateX(-50%);
  padding: 10px 24px; border-radius: 12px; background: rgba(20,24,33,.92); color: #fff;
  font: 700 26px/1.2 -apple-system, BlinkMacSystemFont, "Hiragino Sans", "Segoe UI", sans-serif;
  letter-spacing: 0.08em; pointer-events: none; transition: opacity 400ms; box-shadow: 0 6px 24px rgba(0,0,0,.35); }
#demo-key.off { opacity: 0; }
`;

const CURSOR_SVG = `<svg width="22" height="22" viewBox="0 0 24 24"><path d="M4 2 L4 20 L9 15 L12.5 22 L15.5 20.6 L12 13.8 L19 13.8 Z"
  fill="#111" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>`;

export async function open(url, { lang }) {
  const browser = await chromium.launch({ channel: 'chrome' });
  const context = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: 2, locale: lang === 'ja' ? 'ja-JP' : 'en-US' });
  const page = await context.newPage();
  // Everything drawn over the page lives in one manual popover, which sits in the top layer: the board's
  // dialogs are modal and live there too, so the layer is raised again whenever one opens.
  const install = async () => page.evaluate(([css, svg]) => {
    if (document.getElementById('demo-layer')) return;
    const style = document.createElement('style'); style.textContent = css; document.head.appendChild(style);
    const layer = document.createElement('div'); layer.id = 'demo-layer'; layer.popover = 'manual';
    document.body.appendChild(layer);
    const c = document.createElement('div'); c.id = 'demo-cursor'; c.innerHTML = svg; layer.appendChild(c);
    const cap = document.createElement('div'); cap.id = 'demo-caption'; layer.appendChild(cap);
    const raise = () => { if (layer.matches(':popover-open')) layer.hidePopover(); layer.showPopover(); };
    window.demoRaise = raise;
    raise();
    new MutationObserver(ms => { if (ms.some(m => m.target !== layer && m.attributeName === 'open')) raise(); })
      .observe(document.body, { attributes: true, subtree: true, attributeFilter: ['open'] });
  }, [OVERLAY_CSS, CURSOR_SVG]);
  page.on('load', () => install().catch(() => {}));
  await page.goto(url);
  await install();
  return { browser, context, page, lang, cursor: { x: W / 2, y: H / 2 } };
}

export const sleep = ms => new Promise(r => setTimeout(r, ms));

/* Frames from the DevTools screencast, each kept with the time it was drawn. */
export async function startRecording(rec, dir) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const cdp = await rec.context.newCDPSession(rec.page);
  const frames = [];
  cdp.on('Page.screencastFrame', async f => {
    const file = path.join(dir, `f${String(frames.length).padStart(6, '0')}.jpg`);
    frames.push({ file, t: f.metadata.timestamp });
    fs.writeFileSync(file, Buffer.from(f.data, 'base64'));
    cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {});
  });
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 92, everyNthFrame: 1 });
  rec.stop = async () => {
    await cdp.send('Page.stopScreencast');
    await sleep(200);
    // The screencast sends a frame only when the page changes: each frame lasts until the next.
    const list = frames.map((f, i) => `file '${path.basename(f.file)}'\nduration ${((frames[i + 1]?.t ?? f.t + 0.5) - f.t).toFixed(4)}`);
    list.push(`file '${path.basename(frames.at(-1).file)}'`);
    fs.writeFileSync(path.join(dir, 'frames.txt'), list.join('\n') + '\n');
    const start = frames[0].t, end = frames.at(-1).t + 0.5;
    const caps = (rec.captions || []).map((c, i, all) => ({ text: c.text, from: Math.max(0, c.t - start),
      to: (all[i + 1]?.t ?? end) - start }));
    fs.writeFileSync(path.join(dir, 'captions.json'), JSON.stringify({ duration: end - start, captions: caps }, null, 1));
    return frames.length;
  };
}

/* A caption is not drawn on the page, where it would cover a dialog's buttons: it is kept with the time it
   changed, and drawn as a band under the video when the clip is put together (captions.mjs). */
export async function caption(rec, text) {
  const t = typeof text === 'string' ? text : text[rec.lang];
  (rec.captions ||= []).push({ t: Date.now() / 1000, text: t || '' });
}

export async function title(rec, text, ms = 2200) {
  const t = typeof text === 'string' ? text : text[rec.lang];
  await rec.page.evaluate(t => {
    const el = document.createElement('div'); el.id = 'demo-title'; el.innerHTML = t; document.getElementById('demo-layer').appendChild(el);
  }, t);
  await sleep(ms);
  await rec.page.evaluate(() => { const el = document.getElementById('demo-title'); el.classList.add('off'); setTimeout(() => el.remove(), 500); });
  await sleep(500);
}

/* Draw a keycap badge over the page, press a key or shortcut, and fade the badge out after ~1 s. */
export async function press(rec, keys, label) {
  const t = typeof label === 'string' ? label : label[rec.lang];
  await rec.page.evaluate(t => {
    const el = document.createElement('div'); el.id = 'demo-key'; el.textContent = t;
    document.getElementById('demo-layer').appendChild(el);
  }, t);
  await rec.page.keyboard.press(keys);
  await sleep(1000);
  await rec.page.evaluate(() => {
    const el = document.getElementById('demo-key');
    if (el) { el.classList.add('off'); setTimeout(() => el.remove(), 400); }
  });
  await sleep(400);
}

/* Glide the drawn cursor to the middle of `locator` (or a point), and wait for it to arrive. */
export async function moveTo(rec, target, ms = 700) {
  let x, y;
  if (target.x != null) ({ x, y } = target);
  else {
    await target.waitFor({ state: 'visible', timeout: 30000 });
    await target.scrollIntoViewIfNeeded();
    const b = await target.boundingBox();
    x = b.x + Math.min(b.width / 2, 60); y = b.y + b.height / 2;
  }
  await rec.page.evaluate(([x, y, ms]) => {
    const c = document.getElementById('demo-cursor');
    c.style.setProperty('--move', ms + 'ms'); c.style.transform = `translate(${x}px, ${y}px)`;
  }, [x, y, ms]);
  await rec.page.mouse.move(x, y);
  rec.cursor = { x, y };
  await sleep(ms + 120);
}

export async function click(rec, target, { pause = 500 } = {}) {
  await moveTo(rec, target);
  await rec.page.evaluate(([x, y]) => {
    const r = document.createElement('div'); r.className = 'demo-ripple'; r.style.left = x + 'px'; r.style.top = y + 'px';
    document.getElementById('demo-layer').appendChild(r); setTimeout(() => r.remove(), 600);
  }, [rec.cursor.x, rec.cursor.y]);
  await (target.x != null ? rec.page.mouse.click(target.x, target.y) : target.click());
  await sleep(pause);
}

export async function typeInto(rec, target, text, delay = 55) {
  await click(rec, target, { pause: 250 });
  await rec.page.keyboard.type(text, { delay });
}

export const visible = (page, selector) => page.locator(selector).filter({ visible: true }).first();
export const byText = (page, text) => page.getByText(text, { exact: false }).filter({ visible: true }).first();

/* The first visible match of `selector` whose middle lies between `from` and `to` (CSS x): the same
   button can be in the list column and in the middle at once. */
export async function inColumn(page, selector, from, to, timeout = 60000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const all = page.locator(selector).filter({ visible: true });
    for (let i = 0; i < await all.count(); i++) {
      const b = await all.nth(i).boundingBox().catch(() => null);
      if (b && b.x + b.width / 2 >= from && b.x + b.width / 2 <= to) return all.nth(i);
    }
    await sleep(250);
  }
  throw new Error(`no ${selector} between x=${from} and x=${to}`);
}
