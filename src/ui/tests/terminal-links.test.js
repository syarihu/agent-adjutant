/* Links in the terminal (`terminalLinkOpens` and `openTerminalLink` in src/ui/terminal.js, and
   how `mountSessionTerminal` hands them to xterm), run with `node --test`. A link opens only on a
   click with the platform's modifier, only for http and https, and in a tab of its own. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const src = fs.readFileSync(path.join(__dirname, '..', 'terminal.js'), 'utf8');
const fns = src.match(/function terminalLinkOpens[\s\S]*?\nfunction openTerminalLink[\s\S]*?\n\}\n/);
assert.ok(fns, 'terminalLinkOpens / openTerminalLink not found in terminal.js');
const mount = src.match(/function mountSessionTerminal[\s\S]*?\n\}\n(?=\n\/\* A terminal is offered)/);
assert.ok(mount, 'mountSessionTerminal not found in terminal.js');

const load = (mac) => {
  const opened = [];
  const ctx = vm.createContext({ IS_MAC: mac, URL, window: { open: (...a) => { opened.push(a); } } });
  vm.runInContext(`${fns[0]}; this.opens = terminalLinkOpens; this.open = openTerminalLink;`, ctx);
  return { opens: ctx.opens, open: ctx.open, opened };
};
const cmd = { metaKey: true, ctrlKey: false };
const ctrl = { metaKey: false, ctrlKey: true };
const plain = { metaKey: false, ctrlKey: false };
const url = 'https://github.com/o/r/pull/1';

test('on macOS a link opens with Cmd+click, and not with a plain click or Ctrl+click', () => {
  const { opens } = load(true);
  assert.strictEqual(opens(cmd, url), true);
  assert.strictEqual(opens(plain, url), false);
  assert.strictEqual(opens(ctrl, url), false);
});

test('off macOS a link opens with Ctrl+click, and not with a plain click or Meta+click', () => {
  const { opens } = load(false);
  assert.strictEqual(opens(ctrl, url), true);
  assert.strictEqual(opens(plain, url), false);
  assert.strictEqual(opens(cmd, url), false);
});

test('only http and https open', () => {
  const { opens } = load(true);
  assert.strictEqual(opens(cmd, 'http://localhost:8080/b/x'), true);
  assert.strictEqual(opens(cmd, url), true);
  for (const uri of ['javascript:alert(1)', 'file:///etc/passwd', 'mailto:a@example.com', 'not a url', '', undefined]) {
    assert.strictEqual(opens(cmd, uri), false, String(uri));
  }
});

test('a link is opened in a tab of its own with no opener, and only when the click allows it', () => {
  const { open, opened } = load(true);
  open(plain, url);
  open(cmd, 'javascript:alert(1)');
  assert.deepStrictEqual(opened, []);
  // xterm's OSC 8 handler passes its range third: it is not read as the platform.
  open(cmd, url, { start: { x: 1, y: 1 }, end: { x: 2, y: 1 } });
  assert.deepStrictEqual(opened, [[url, '_blank', 'noopener,noreferrer']]);
});

test('the terminal hands both plain URLs and OSC 8 links to the same opener', async () => {
  let options = null;
  const addons = [];
  const calls = [];
  class FakeTerminal {
    constructor(o) { options = o; this.cols = 80; this.rows = 24; this.unicode = {}; }
    loadAddon(a) { addons.push(a); }
    open() {}
    onData() {}
    onBinary() {}
    onResize() {}
    attachCustomKeyEventHandler() {}
    focus() {}
    write() {}
    dispose() {}
  }
  class WebLinks { constructor(handler) { this.handler = handler; } }
  const ctx = vm.createContext({
    loadXterm: () => Promise.resolve(),
    Terminal: FakeTerminal,
    IS_MAC: true,
    FitAddon: { FitAddon: class { fit() {} } },
    Unicode11Addon: { Unicode11Addon: class {} },
    WebLinksAddon: { WebLinksAddon: WebLinks },
    openTerminalLink: (...a) => calls.push(a),
    WebSocket: class { static OPEN = 1; send() {} close() {} },
    ResizeObserver: class { observe() {} disconnect() {} },
    TextEncoder,
    requestAnimationFrame: () => 1,
    cancelAnimationFrame() {},
    location: { protocol: 'http:', host: 'x' },
    TOKEN: 't',
    BASE: '',
    JSON,
    encodeURIComponent,
  });
  vm.runInContext(`${mount[0]}; this.mount = mountSessionTerminal;`, ctx);
  ctx.mount({ classList: { add() {}, remove() {} }, textContent: '', clientWidth: 1, clientHeight: 1 }, { sessionId: 's' });
  await new Promise(r => setImmediate(r));
  assert.strictEqual(options.linkHandler.allowNonHttpProtocols, false);
  options.linkHandler.activate(cmd, url, { start: { x: 1, y: 1 }, end: { x: 2, y: 1 } });
  const web = addons.find(a => a instanceof WebLinks);
  assert.ok(web, 'the web-links addon is loaded');
  web.handler(cmd, url);
  assert.deepStrictEqual(calls.map(c => c.slice(0, 2)), [[cmd, url], [cmd, url]]);
});
