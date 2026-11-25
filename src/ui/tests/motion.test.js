/* The pure parts of the page's motion (src/ui/motion.js): what arrived and what changed stage between two drawings, and what a
   drawing remembers. The file does nothing at load, so it is run in a context with no document. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({});
vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'motion.js'), 'utf8') + '\nthis.api = { diffKeys, stageDiff, motionTrack, motionForget, motionCountedUp, motionCalm };', ctx, { filename: 'motion.js' });
const { diffKeys, stageDiff, motionTrack, motionForget, motionCountedUp, motionCalm } = ctx.api;
const plain = x => JSON.parse(JSON.stringify(x));
const stages = o => new Map(Object.entries(o));

test('the first drawing, or one after a reset, has nothing arriving', () => {
  assert.deepStrictEqual(plain(diffKeys(null, ['a', 'b'])), []);
  assert.deepStrictEqual(plain(diffKeys(new Set(), ['a', 'b'])), []);
  assert.deepStrictEqual(plain(stageDiff(null, stages({ 'x|1': 'me' }))), { appeared: [], moved: [], finished: [] });
  assert.deepStrictEqual(plain(stageDiff(new Map(), stages({ 'x|1': 'me' }))), { appeared: [], moved: [], finished: [] });
});

test('diffKeys gives the keys that were not there before, and ignores the ones that went', () => {
  assert.deepStrictEqual(plain(diffKeys(new Set(['a', 'b']), ['b', 'c', 'd'])), ['c', 'd']);
});

test('stageDiff names what appeared, what moved and what moved into done', () => {
  const prev = stages({ 'x|1': 'me', 'x|2': 'working', 'x|3': 'me', 'x|4': 'before' });
  const next = stages({ 'x|1': 'me', 'x|2': 'done', 'x|3': 'working', 'x|5': 'before' });
  assert.deepStrictEqual(plain(stageDiff(prev, next)), {
    appeared: ['x|5'],
    moved: [{ key: 'x|2', from: 'working', to: 'done' }, { key: 'x|3', from: 'me', to: 'working' }],
    finished: ['x|2'],
  });
});

test('a task that arrives already done is not finished, and one that was removed is ignored', () => {
  const out = stageDiff(stages({ 'x|1': 'me', 'x|2': 'me' }), stages({ 'x|1': 'me', 'x|9': 'done' }));
  assert.deepStrictEqual(plain(out), { appeared: ['x|9'], moved: [], finished: [] });
});

test('a drawing remembers its last map only for the same sig', () => {
  motionForget('t');
  assert.strictEqual(motionTrack('t', 'org-a', new Map([['k', 1]])), null);
  assert.deepStrictEqual(plain([...motionTrack('t', 'org-a', new Map())]), [['k', 1]]);
  motionTrack('t', 'org-a', new Map([['k', 1]]));
  assert.strictEqual(motionTrack('t', 'org-b', new Map()), null);
  motionForget('t');
  assert.strictEqual(motionTrack('t', 'org-b', new Map()), null);
});

test('a count bumps only when it goes up from a value read under the same sig', () => {
  motionForget('c');
  assert.strictEqual(motionCountedUp('c', 'a', 3), false);
  assert.strictEqual(motionCountedUp('c', 'a', 4), true);
  assert.strictEqual(motionCountedUp('c', 'a', 4), false);
  assert.strictEqual(motionCountedUp('c', 'a', 2), false);
  assert.strictEqual(motionCountedUp('c', 'b', 9), false);
});

test('with no Element.animate the page is calm', () => {
  assert.strictEqual(motionCalm(), true);
});
