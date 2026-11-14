/* The terminal's options in the desktop app (appShellTerminalOptions in src/ui/terminal.js), run with `node --test`.
   Only the app, which stamps `data-shell="app"` on <html>, gets a transparent terminal; a browser gets nothing. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const block = src.match(/function appShellTerminalOptions\([\s\S]*?\n\}\n/);
assert.ok(block, 'appShellTerminalOptions not found in terminal.js');

const options = shell => {
  const root = { getAttribute: name => (name === 'data-shell' ? shell : null) };
  const ctx = vm.createContext({ document: { documentElement: root } });
  vm.runInContext(block[0], ctx);
  // Through JSON, so the object made in the other context compares with deepStrictEqual.
  return JSON.parse(JSON.stringify(vm.runInContext('appShellTerminalOptions()', ctx)));
};

test('a browser gets no options, so its terminal renders as before', () => {
  assert.deepStrictEqual(options(null), {});
});

test('another shell value is not the app', () => {
  assert.deepStrictEqual(options('web'), {});
});

test('the app gets a terminal with a transparent background and a visible block cursor', () => {
  const o = options('app');
  assert.strictEqual(o.allowTransparency, true);
  assert.strictEqual(o.theme.background, 'rgba(0, 0, 0, 0)');
  assert.strictEqual(o.theme.cursorAccent, '#000000');
});

test('the terminal is created with them', () => {
  assert.match(src, /new Terminal\(\{[\s\S]*?\.\.\.appShellTerminalOptions\(\),[\s\S]*?\}\);/);
});

/* The stylesheet is the app's alone: a rule a browser could match would change how the board looks there. */
const css = fs.readFileSync(path.join(__dirname, '..', 'app-shell.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const selectors = [...css.matchAll(/([^{}]+)\{/g)]
  .map(m => m[1].trim())
  .filter(s => !s.startsWith('@'))
  .flatMap(s => s.split(',').map(part => part.trim()));

test('every rule in app-shell.css is scoped to the app', () => {
  assert.ok(selectors.length > 0);
  for (const s of selectors) assert.ok(s.startsWith('[data-shell="app"]'), `unscoped selector: ${s}`);
});

test('the app clears the black xterm.css gives the terminal viewport', () => {
  assert.ok(selectors.includes('[data-shell="app"] .adj-terminal .xterm-viewport'));
  assert.match(css, /\.adj-terminal \.xterm-viewport \{ background-color: transparent; \}/);
});
