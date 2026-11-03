// node run.mjs <clip> <ja|en> <board url> <frames dir>: play one clip on a fresh demo world.
import { open, startRecording } from './lib.mjs';
const [clipName, lang, url, dir] = process.argv.slice(2);
const clip = await import(`./clips/${clipName}.mjs`);
const rec = await open(url, { lang });
await rec.page.waitForTimeout(2500);
await startRecording(rec, dir);
try { await clip.play(rec); }
catch (e) { await rec.page.screenshot({ path: `${dir}/../${clipName}-${lang}-error.png` }); throw e; }
finally { console.log('frames', await rec.stop()); await rec.browser.close(); }
