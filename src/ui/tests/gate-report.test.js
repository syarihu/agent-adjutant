/* The page's side of the HTML report a gate carries (src/ui/gate-report.js): the document handed to the sandboxed frame. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
for (const file of ['util.js', 'gate-report.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const { reportDocument, REPORT_CSP } = vm.runInContext('({ reportDocument, REPORT_CSP })', ctx);
const meta = `<meta http-equiv="Content-Security-Policy" content="${REPORT_CSP.replace(/'/g, '&#39;')}">`;

test('the policy goes right after the doctype, which stays first', () => {
  assert.equal(reportDocument('<!DOCTYPE html><html></html>'), `<!DOCTYPE html>${meta}<html></html>`);
});

test('whitespace before the doctype stays before it', () => {
  const html = '\n  <!doctype html>\n<html></html>';
  assert.equal(reportDocument(html), `\n  <!doctype html>${meta}\n<html></html>`);
});

test('a comment first puts the policy at the very start, so no comment can lead past a script to a later doctype', () => {
  for (const html of [
    '<!-- made by a worker -->\n<!doctype html>\n<html></html>',
    '<!--x--><script src="https://example.com/x.js"></script><!-- --><!doctype html><p>hi</p>',
    '<!--><script src="https://example.com/x.js"></script><!-- --><!doctype html><p>hi</p>',
  ]) assert.equal(reportDocument(html), meta + html);
});

test('a report with no doctype gets the policy at the very start', () => {
  assert.equal(reportDocument('<p>hi</p>'), `${meta}<p>hi</p>`);
});

test('the policy comes before a script the report loads from elsewhere', () => {
  const out = reportDocument('<!doctype html><script src=https://example.com/x.js></script>');
  assert.ok(out.indexOf('Content-Security-Policy') < out.indexOf('example.com'));
});

test('the quotes in the policy cannot break the attribute', () => {
  const out = reportDocument('<p>hi</p>');
  const content = /content="([^"]*)"/.exec(out)[1];
  assert.ok(content.includes('default-src &#39;none&#39;'));
  assert.ok(!content.includes('"'));
});

test('a doctype word further down is not mistaken for the doctype', () => {
  const html = '<p>a</p><!doctype html>';
  assert.equal(reportDocument(html), `${meta}${html}`);
});
