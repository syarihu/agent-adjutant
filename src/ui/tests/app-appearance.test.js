/* The app's window settings in the display settings (src/ui/app-appearance.js), run with `node --test`.
   A browser, and an app that does not hand the page its appearance, keep the section hidden; the app shows it, draws
   its values and sends what the person changes to the `set_appearance` command. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'app-appearance.js'), 'utf8');
const html = fs.readFileSync(path.join(__dirname, '..', 'page-body.html'), 'utf8');

class FakeTarget {
  constructor() { this.listeners = {}; }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  fire(type, event = {}) { for (const fn of this.listeners[type] || []) fn(event); }
}

function page({ shell = null, appearance, tauri = true } = {}) {
  const calls = [];
  const radios = ['material', 'none'].map(value => Object.assign(new FakeTarget(), { value, checked: false }));
  const slider = Object.assign(new FakeTarget(), { value: '50', min: '0', max: '100' });
  const output = { textContent: '' };
  const section = { hidden: true, querySelectorAll: () => radios };
  const style = {};
  const attrs = {};
  const root = {
    getAttribute: name => (name === 'data-shell' ? shell : (name in attrs ? attrs[name] : null)),
    setAttribute: (name, value) => { attrs[name] = String(value); },
    removeAttribute: name => { delete attrs[name]; },
    style: { setProperty: (k, v) => { style[k] = v; } },
  };
  const error = { hidden: true, textContent: '' };
  const byId = {
    'display-app': section, 'display-app-opacity': slider, 'display-app-opacity-value': output, 'display-app-error': error,
  };
  const win = new FakeTarget();
  if (appearance) win.__adjAppearance = appearance;
  if (tauri) win.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => { calls.push([cmd, args]); } };
  const doc = { documentElement: root, getElementById: id => byId[id] || null };
  const ctx = vm.createContext({ document: doc, window: win, console });
  vm.runInContext(src, ctx);
  return { section, radios, slider, output, style, win, calls, error, root };
}

const APP = { background: 'material', alpha: 0.82, minAlpha: 0.3, maxAlpha: 1 };
const settle = () => new Promise(resolve => setImmediate(resolve));

test('the section is hidden in the markup, so a browser never shows it', () => {
  const tag = html.match(/<fieldset[^>]*id="display-app"[^>]*>/);
  assert.ok(tag, 'the app section is not in page-body.html');
  assert.match(tag[0], /\shidden[\s>]/);
});

test('a browser keeps the section hidden', () => {
  assert.strictEqual(page().section.hidden, true);
});

test('an app that does not hand over its appearance keeps it hidden', () => {
  assert.strictEqual(page({ shell: 'app' }).section.hidden, true);
  assert.strictEqual(page({ shell: 'app', appearance: APP, tauri: false }).section.hidden, true);
});

test('the app shows the section with its values', () => {
  const p = page({ shell: 'app', appearance: APP });
  assert.strictEqual(p.section.hidden, false);
  assert.deepStrictEqual(p.radios.map(r => r.checked), [true, false]);
  assert.strictEqual(p.slider.min, '30');
  assert.strictEqual(p.slider.max, '100');
  assert.strictEqual(p.slider.value, '82');
  assert.strictEqual(p.output.textContent, '82%');
});

test('choosing a background sends it and saves it', async () => {
  const p = page({ shell: 'app', appearance: APP });
  p.radios[1].checked = true;
  p.radios[1].fire('change');
  await settle();
  assert.deepStrictEqual(JSON.parse(JSON.stringify(p.calls)),
    [['set_appearance', { background: 'none', persist: true }]]);
});

test('the slider shows its value at once and saves it when let go', async () => {
  const p = page({ shell: 'app', appearance: APP });
  p.slider.fire('pointerdown');
  p.slider.value = '60';
  p.slider.fire('input');
  assert.strictEqual(p.style['--app-alpha'], '0.6');
  assert.strictEqual(p.output.textContent, '60%');
  p.slider.fire('change');
  await settle();
  const calls = JSON.parse(JSON.stringify(p.calls));
  assert.deepStrictEqual(calls[0], ['set_appearance', { opacity: 0.6, persist: false }]);
  assert.deepStrictEqual(calls[calls.length - 1], ['set_appearance', { opacity: 0.6, persist: true }]);
});

test('a change from the menu is drawn, but not over a slider being dragged', () => {
  const p = page({ shell: 'app', appearance: APP });
  p.win.fire('adj-appearance', { detail: { ...APP, background: 'none', alpha: 0.5 } });
  assert.deepStrictEqual(p.radios.map(r => r.checked), [false, true]);
  assert.strictEqual(p.slider.value, '50');
  p.slider.fire('pointerdown');
  p.slider.value = '70';
  p.win.fire('adj-appearance', { detail: { ...APP, alpha: 0.4 } });
  assert.strictEqual(p.slider.value, '70');
});

test('the default and the old levels land on a step of the slider, so it shows them as they are', () => {
  const tag = html.match(/<input[^>]*id="display-app-opacity"[^>]*>/)[0];
  const attr = name => Number(tag.match(new RegExp(`\\s${name}="([^"]+)"`))[1]);
  const [min, max, step] = [attr('min'), attr('max'), attr('step')];
  for (const alpha of [0.82, 1, 0.55, 0.3]) {
    const percent = Math.round(alpha * 100);
    assert.ok(percent >= min && percent <= max, `${percent} outside the slider`);
    assert.strictEqual((percent - min) % step, 0, `${percent}% is not on a step`);
  }
});

test('steps sent while one is on its way fold into the latest, and its echoes do not move the slider', async () => {
  const p = page({ shell: 'app', appearance: APP });
  let release;
  const calls = [];
  p.win.__TAURI_INTERNALS__.invoke = (cmd, args) => {
    calls.push(JSON.parse(JSON.stringify(args)));
    return new Promise(resolve => { release = resolve; });
  };
  for (const value of ['81', '80', '79']) {
    p.slider.value = value;
    p.slider.fire('input');
  }
  p.slider.fire('change');
  // The app echoes the first step back while the rest wait: the slider stays where the keys put it.
  p.win.fire('adj-appearance', { detail: { ...APP, alpha: 0.81 } });
  assert.strictEqual(p.slider.value, '79');
  release();
  await settle();
  assert.deepStrictEqual(calls, [{ opacity: 0.81, persist: false }, { opacity: 0.79, persist: true }]);
  release();
  await settle();
});

test('a save that fails is shown, and a later save that goes through clears it', async () => {
  const p = page({ shell: 'app', appearance: APP });
  p.win.__TAURI_INTERNALS__.invoke = async () => { throw 'disk full'; };
  p.radios[1].checked = true;
  p.radios[1].fire('change');
  await settle();
  assert.strictEqual(p.error.hidden, false);
  assert.match(p.error.textContent, /保存できませんでした.*disk full/);
  p.win.__TAURI_INTERNALS__.invoke = async () => {};
  p.slider.value = '70';
  p.slider.fire('input');
  await settle();
  assert.strictEqual(p.error.hidden, false, 'a step that is not saved leaves the error up');
  p.slider.fire('change');
  await settle();
  assert.strictEqual(p.error.hidden, true);
});

test('the background is mirrored onto the page for the board styles, at startup and on every change', () => {
  const bg = p => p.root.getAttribute('data-app-background');
  assert.strictEqual(bg(page({ shell: 'app', appearance: APP })), 'material');
  assert.strictEqual(bg(page({ shell: 'app', appearance: { ...APP, background: 'none' } })), 'none');
  const p = page({ shell: 'app', appearance: APP });
  p.win.fire('adj-appearance', { detail: { ...APP, background: 'none' } });
  assert.strictEqual(bg(p), 'none');
  p.win.fire('adj-appearance', { detail: { ...APP, background: 'material' } });
  assert.strictEqual(bg(p), 'material');
});

test('an app that cannot send changes still mirrors its background', () => {
  const p = page({ shell: 'app', appearance: APP, tauri: false });
  assert.strictEqual(p.root.getAttribute('data-app-background'), 'material');
  assert.strictEqual(p.section.hidden, true);
});

test('a browser gets no background attribute, and an unknown value takes it off', () => {
  assert.strictEqual(page({ appearance: APP }).root.getAttribute('data-app-background'), null);
  const p = page({ shell: 'app', appearance: APP });
  p.win.fire('adj-appearance', { detail: { ...APP, background: 'glass' } });
  assert.strictEqual(p.root.getAttribute('data-app-background'), null);
});
