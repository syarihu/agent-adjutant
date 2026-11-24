/* The groups of 「いまの仕事」 (src/ui/my-work.css, src/ui/tokens.css), run with `node --test`. A group, its heading or its body must
   not become a scroll container, or the repository heading stops sticking to the top of the list. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const strip = text => text.replace(/\/\*[\s\S]*?\*\//g, '');
const css = strip(fs.readFileSync(path.join(__dirname, '..', 'my-work.css'), 'utf8'));
const tokens = strip(fs.readFileSync(path.join(__dirname, '..', 'tokens.css'), 'utf8'));

test('no rule of a .wk-group, .wk-head or .wk-body sets overflow, contain or transform', () => {
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(m => /\.wk-(group|head|body)\b(?!-)/.test(m[1]));
  assert.ok(rules.length > 0, 'the group rules are found');
  for (const [, selector, body] of rules) {
    assert.doesNotMatch(body, /(^|[;\s])(overflow(-[xy])?|contain|transform)\s*:/, `${selector.trim()} must not set overflow, contain or transform`);
  }
});

test('the list column keeps the width the person set in 「いまの仕事」 at every width', () => {
  assert.doesNotMatch(css, /view-work\s+#list-resize\s*\{\s*display:\s*none/);
  assert.doesNotMatch(css, /view-work\s*\{\s*--list-w:\s*(240px|max\()/);
});

test('a row\'s title is not tinted by its state; the state is told by its icon and chip', () => {
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(m => /\.wk-row\.(waiting|permission|done)\s+\.wk-title\b/.test(m[1]));
  for (const [, selector, body] of rules) {
    assert.doesNotMatch(body, /(^|[;\s])color\s*:\s*var\(--md-sys-color-(warning|error|success)\)/, `${selector.trim()} must not tint the title`);
  }
});

test('tokens.css defines every typescale, motion and state token my-work.css uses', () => {
  const used = new Set(css.match(/var\(--md-sys-(typescale|motion|state)-[\w-]+/g)?.map(v => v.slice(4)));
  assert.ok(used.size > 0, 'the tokens are used');
  for (const name of used) assert.match(tokens, new RegExp(`${name}\\s*:`), `${name} is defined in tokens.css`);
});

test('the rule between rows is drawn under a plain row as well as under a wrapped one', () => {
  const rule = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(m => /::after/.test(m[1]) && /background:\s*var\(--wk-rule\)/.test(m[2]));
  assert.ok(rule, 'the divider rule is found');
  assert.match(rule[1], /\.wk-body\s*>\s*:is\(\.wk-row,\s*\.wk-band-row\)::after/, 'it covers a bare .wk-row and a .wk-band-row');
});

test('no selector nests :has() inside :has(), which CSS does not allow', () => {
  // Walk each selector's parentheses, remembering which open ones are a :has(.
  const nests = selector => {
    const open = [];
    for (let i = 0; i < selector.length; i++) {
      if (selector[i] === '(') {
        const has = selector.slice(Math.max(0, i - 4), i) === ':has';
        if (has && open.includes(true)) return true;
        open.push(has);
      } else if (selector[i] === ')') open.pop();
    }
    return false;
  };
  assert.ok(nests('.a:has(+ :is(.b, .c:has(> .d)))'), 'the check sees a nested :has()');
  for (const [, selector] of css.matchAll(/([^{}]+)\{[^{}]*\}/g)) assert.ok(!nests(selector), `${selector.trim()} nests :has()`);
});
