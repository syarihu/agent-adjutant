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

const ruleOf = (selector, where = css) => [...where.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(m => m[1].trim() === selector)?.[2];

test('a row with things beside it is toned as one container, so its count, fold and buttons sit inside the tone', () => {
  const box = ':is(.wk-band-row:not(.ov-row-wrap), .wk-hubhead)';
  assert.ok(ruleOf(`${box}:hover`), 'hover tones the container');
  assert.match(ruleOf(`${box}:has(.wk-row[aria-current="true"])`) || '', /background:\s*var\(--md-sys-color-secondary-container\)/, 'selection tones the container');
  assert.match(ruleOf(`${box} .wk-row:is(:hover, :active, [aria-current="true"], [aria-current="true"]:hover)`) || '', /background:\s*transparent/, 'the row inside draws no tone of its own');
  // The hub's own wrapper inside a group's head stays clear, or the head would be toned twice.
  assert.ok([...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].some(m => /^\.wk-hubhead > \.wk-band-row/.test(m[1].trim()) && /background:\s*transparent/.test(m[2])));
});

test('the count of a hub that heads a group is as wide as the fold that takes its place on hover', () => {
  const count = ruleOf('.wk-hubhead .wk-count') || '';
  const fold = ruleOf('.wk-fold') || '';
  const width = fold.match(/(?:^|[;\s])width:\s*(\d+px)/)?.[1];
  assert.ok(width, 'the fold has a width');
  assert.match(count, new RegExp(`min-width:\\s*${width}`));
});

test('a row\'s state chip stays on one line and is cut with an ellipsis', () => {
  const chip = ruleOf('.wk-gate, .wk-line.ask') || '';
  assert.match(chip, /white-space:\s*nowrap/);
  assert.match(chip, /text-overflow:\s*ellipsis/);
  assert.match(chip, /overflow:\s*hidden/);
  assert.match(chip, /max-width:\s*100%/);
  // text-overflow does not cut the text of a flex box.
  assert.doesNotMatch(chip, /display:\s*inline-flex/);
});

test('reduced motion stills a row container with a selector as specific as the one that animates it', () => {
  const box = ':is(.wk-band-row:not(.ov-row-wrap), .wk-hubhead)';
  const reduced = css.match(/@media \(prefers-reduced-motion: reduce\) \{([\s\S]*?\n)\}/g)?.join('\n') || '';
  const rules = [...reduced.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(m => [m[1].split(',').map(x => x.trim()), m[2]]);
  // A selector is split on its commas, so the container's :is( list comes back joined.
  const has = (sel, prop) => rules.some(([sels, body]) => sels.join(', ').includes(sel) && new RegExp(`${prop}:\\s*none`).test(body));
  assert.ok(has(box, 'transition'), 'the container transition is turned off');
  assert.ok(has(`${box}:has(.wk-row:active)`, 'transform'), 'the container press does not scale');
});

test('the count of a selected hub head takes the selection tone\'s text colour', () => {
  assert.match(ruleOf('.wk-hubhead:has(.wk-row[aria-current="true"]) .wk-count') || '', /color:\s*inherit/);
});
