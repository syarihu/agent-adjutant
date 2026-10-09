/* The group cards of 「いまの仕事」 (src/ui/my-work.css), run with `node --test`. A card, its heading or its body must
   not become a scroll container, or the repository heading stops sticking to the top of the list. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const css = fs.readFileSync(path.join(__dirname, '..', 'my-work.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

test('no rule of a .wk-group, .wk-head or .wk-body sets overflow, contain or transform', () => {
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(m => /\.wk-(group|head|body)\b(?!-)/.test(m[1]));
  assert.ok(rules.length > 0, 'the card rules are found');
  for (const [, selector, body] of rules) {
    assert.doesNotMatch(body, /(^|[;\s])(overflow(-[xy])?|contain|transform)\s*:/, `${selector.trim()} must not set overflow, contain or transform`);
  }
});
