/* The paste of the terminal handle (mountSessionTerminal in src/ui/terminal.js), run with `node --test`.
   The text goes in through xterm's own paste, so the program sees one event, and only while the
   socket is open. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const block = src.match(/function mountSessionTerminal[\s\S]*?\n\}\n(?=\n\/\* A terminal is offered)/);
assert.ok(block, 'mountSessionTerminal not found in terminal.js');

const setup = () => {
  const pasted = [];
  const sent = [];
  const sockets = [];
  class FakeTerminal {
    constructor() { this.cols = 80; this.rows = 24; this.unicode = {}; }
    loadAddon() {}
    open() {}
    onData() {}
    onBinary() {}
    onResize() {}
    focus() {}
    write() {}
    paste(t) { pasted.push(t); }
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
    FitAddon: { FitAddon: class { fit() {} } },
    Unicode11Addon: { Unicode11Addon: class {} },
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
  vm.runInContext(`${block[0]}; this.mount = mountSessionTerminal;`, ctx);
  const handle = ctx.mount(container, { sessionId: 's1', focus: false });
  return { handle, pasted, sent, sockets };
};
const ready = () => new Promise(r => setImmediate(r));

test('paste is refused before the terminal and the socket exist', async () => {
  const r = setup();
  assert.strictEqual(r.handle.paste('a'), false);
  await ready();
  assert.strictEqual(r.handle.paste('a'), false, 'the socket is still connecting');
  assert.deepStrictEqual(r.pasted, []);
});

test('paste goes through the terminal once the socket is open', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  assert.strictEqual(r.handle.paste('one\ntwo'), true);
  assert.deepStrictEqual(r.pasted, ['one\ntwo']);
});

test('paste is refused after the terminal is closed', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  r.handle.dispose();
  assert.strictEqual(r.handle.paste('a'), false);
  assert.deepStrictEqual(r.pasted, []);
});

test('enter sends the Enter key alone', async () => {
  const r = setup();
  await ready();
  r.sockets[0].readyState = 1;
  assert.strictEqual(r.handle.enter(), true);
  assert.deepStrictEqual(Array.from(r.sent.at(-1)), [13]);
});
