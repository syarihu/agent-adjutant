/* How the board says a child comes after siblings it is not stacked on (`after` and `afterHub` of a child of a
   parent, from src/board/view/parents.rs): the pure helpers afterTitlesOf and afterText in src/ui/util.js only.
   The parent's detail dialog using them is tested in my-work-tree.test.js; the task panel's row is not tested. Run with `node --test`. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'util.js'), 'utf8'), ctx, { filename: 'util.js' });
const { afterTitlesOf, afterText } = ctx;

const kids = [
  { id: 'api', hub: 'h1', title: 'API' },
  { id: 'api', hub: 'h2', title: 'Other hub API' },
  { id: 'db', hub: 'h1', title: '' },
  { id: 'screen', hub: 'h1', title: 'Screen', after: ['api', 'db'], afterHub: ['h2', 'h1'] },
];

test('the siblings a child comes after are found by id and hub, in the order written', () => {
  assert.deepStrictEqual(Array.from(afterTitlesOf(kids, kids[3])), ['Other hub API', '#db']);
});

test('a sibling the group does not list is left out, and a child with none has none', () => {
  assert.deepStrictEqual(Array.from(afterTitlesOf(kids, { id: 'x', after: ['gone'], afterHub: ['h1'] })), []);
  assert.deepStrictEqual(Array.from(afterTitlesOf(kids, kids[0])), []);
  assert.deepStrictEqual(Array.from(afterTitlesOf(kids, undefined)), []);
});

test('a caller can give the title the page knows', () => {
  assert.deepStrictEqual(Array.from(afterTitlesOf(kids, kids[3], c => `row ${c.id}`)), ['row api', 'row db']);
});

test('the phrase names every title, and is empty for none', () => {
  assert.strictEqual(afterText(['API', 'DB']), '「API」「DB」の後');
  assert.strictEqual(afterText([]), '');
});
