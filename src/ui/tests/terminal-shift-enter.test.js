/* Shift+Enter in the terminal (`shiftEnterKey` and the key handler `mountSessionTerminal` in
   src/ui/terminal.js gives xterm), run with `node --test`. The keydown sends ESC CR, the keypress
   after it is swallowed so xterm does not also send CR, and every other key is left to xterm. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const fns = src.match(/const SHIFT_ENTER_SEQUENCE = [\s\S]*?\nfunction shiftEnterKey[\s\S]*?\n\}\n/);
assert.ok(fns, 'shiftEnterKey not found in terminal.js');
const mountBlock = src.match(/function mountSessionTerminal[\s\S]*?\n\}\n(?=\n\/\* A terminal is offered)/);
assert.ok(mountBlock, 'mountSessionTerminal not found in terminal.js');

const key = (over = {}) => ({
  type: 'keydown', key: 'Enter', shiftKey: true, ctrlKey: false, altKey: false, metaKey: false,
  isComposing: false, keyCode: 13, ...over,
});

const pure = vm.createContext({});
vm.runInContext(`${fns[0]}; this.take = shiftEnterKey;`, pure);
const take = e => pure.take(e);

test('Shift+Enter is sent on keydown and swallowed on keypress and keyup', () => {
  assert.strictEqual(take(key()), 'send');
  assert.strictEqual(take(key({ type: 'keypress' })), 'swallow');
  assert.strictEqual(take(key({ type: 'keyup' })), 'swallow');
});

test('other keys are left to xterm', () => {
  assert.strictEqual(take(key({ shiftKey: false })), null, 'plain Enter');
  assert.strictEqual(take(key({ ctrlKey: true })), null);
  assert.strictEqual(take(key({ altKey: true })), null);
  assert.strictEqual(take(key({ metaKey: true })), null);
  assert.strictEqual(take(key({ key: 'a' })), null, 'Shift+A');
});

test('an Enter that confirms an IME conversion is left to xterm', () => {
  assert.strictEqual(take(key({ isComposing: true })), null);
  assert.strictEqual(take(key({ keyCode: 229 })), null);
});

const setup = ({ mac = false, palette = false } = {}) => {
  const sent = [];
  const sockets = [];
  const palettes = [];
  const handlers = [];
  class FakeTerminal {
    constructor() { this.cols = 80; this.rows = 24; this.unicode = {}; }
    loadAddon() {}
    open() {}
    onData() {}
    onBinary() {}
    onResize() {}
    attachCustomKeyEventHandler(h) { handlers.push(h); }
    focus() {}
    write() {}
    dispose() {}
  }
  class FakeSocket {
    static OPEN = 1;
    constructor() { this.readyState = 0; sockets.push(this); }
    send(d) { sent.push(d); }
    close() { this.readyState = 3; }
  }
  const container = { classList: { add() {}, remove() {} }, textContent: '', clientWidth: 1, clientHeight: 1 };
  const ctx = vm.createContext({
    loadXterm: () => Promise.resolve(),
    Terminal: FakeTerminal,
    appShellTerminalOptions: () => ({}),
    IS_MAC: mac,
    isPaletteShortcut: e => e.metaKey && e.key === 'k',
    openPaletteFromKey: () => { palettes.push(1); return palette; },
    FitAddon: { FitAddon: class { fit() {} } },
    Unicode11Addon: { Unicode11Addon: class {} },
    WebLinksAddon: { WebLinksAddon: class {} },
    TERMINAL_URL: /x/,
    openTerminalLink() {},
    WebSocket: FakeSocket,
    ResizeObserver: class { observe() {} disconnect() {} },
    TextEncoder,
    Uint8Array,
    requestAnimationFrame: () => 1,
    cancelAnimationFrame() {},
    location: { protocol: 'http:', host: 'x' },
    TOKEN: 't',
    BASE: '',
    JSON,
    encodeURIComponent,
  });
  vm.runInContext(`${fns[0]}; ${mountBlock[0]}; this.mount = mountSessionTerminal;`, ctx);
  ctx.mount(container, { sessionId: 's1', focus: false });
  return { handlers, sent, sockets, palettes };
};
const ready = () => new Promise(r => setImmediate(r));
const event = over => {
  const e = key(over);
  e.prevented = 0;
  e.preventDefault = () => { e.prevented++; };
  return e;
};
const bytes = d => Array.from(d);
const sentBytes = r => r.sent.filter(d => d instanceof Uint8Array).map(bytes);

test('Shift+Enter keydown sends ESC CR on the open socket, and xterm does not get the key', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  const e = event();
  assert.strictEqual(r.handlers[0](e), false);
  assert.strictEqual(e.prevented, 1);
  assert.deepStrictEqual(sentBytes(r), [[0x1b, 0x0d]]);
});

test('the keypress after Shift+Enter returns false and sends nothing', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  const e = event({ type: 'keypress' });
  assert.strictEqual(r.handlers[0](e), false);
  assert.deepStrictEqual(sentBytes(r), []);
});

test('Shift+Enter sends nothing while the socket is not open, and is still kept from xterm', async () => {
  const r = setup();
  assert.strictEqual(r.handlers.length, 0, 'the terminal is not created before loadXterm resolves');
  await ready();
  const e = event();
  assert.strictEqual(r.handlers[0](e), false);
  assert.deepStrictEqual(sentBytes(r), []);
});

test('plain Enter goes to xterm and sends nothing', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  const e = event({ shiftKey: false });
  assert.strictEqual(r.handlers[0](e), true);
  assert.strictEqual(e.prevented, 0);
  assert.deepStrictEqual(sentBytes(r), []);
});

test('Cmd+K on macOS still opens the palette, and is kept from xterm', async () => {
  const r = setup({ mac: true, palette: true });
  await ready();
  const e = event({ key: 'k', shiftKey: false, metaKey: true });
  assert.strictEqual(r.handlers[0](e), false);
  assert.strictEqual(r.palettes.length, 1);
  assert.deepStrictEqual(sentBytes(r), []);
});

test('Cmd+K with another dialog open goes to xterm', async () => {
  const r = setup({ mac: true, palette: false });
  await ready();
  assert.strictEqual(r.handlers[0](event({ key: 'k', shiftKey: false, metaKey: true })), true);
});
