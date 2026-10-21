/* The annotated diff as data and markup (src/ui/pr-diff.js), run with `node --test`. The page's scripts share one global scope,
   so they are loaded into one context the way the browser does. */
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ctx = vm.createContext({
  TextEncoder, TextDecoder,
  secTitle: text => `<div class="tp-sec-title">${text}</div>`,
  panelBtnsHtml: jump => `<div class="tp-head-btns">${jump}<button data-tp-close></button></div>`,
});
for (const file of ['util.js', 'my-work-seen.js', 'my-work-others.js', 'pr-read.js', 'pr-diff.js']) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), ctx, { filename: file });
}
const plain = x => JSON.parse(JSON.stringify(x));
const prdBoxKey = vm.runInContext('prdBoxKey', ctx);
const { prParseDiff, prDiffCategory, prDiffIsGenerated, prDiffLayout, prDiffAnchor, prDiffCommentBlock, prDiffResolve, prDiffDraft, prDiffSideOf,
  prDiffShownNo, prDiffFindLine, prDiffFindAt, prDiffPlaceOf, prDiffUnplaced, prdCommentHtml, prdCommentItems,
  prDiffBarHtml, prDiffShellHtml, prDiffBodyHtml, prDiffLabels, prDiffAtOf, prdAnnHtml, prDiffExcerptHtml } = ctx;

const SAMPLE = [
  'diff --git a/src/a.rs b/src/a.rs',
  'index 1111111..2222222 100644',
  '--- a/src/a.rs',
  '+++ b/src/a.rs',
  '@@ -1,3 +1,4 @@ fn a',
  ' fn a() {',
  '-    old();',
  '+    // why',
  '+    new();',
  ' }',
  '@@ -10 +11 @@ fn b',
  '-x',
  '+y',
  '\\ No newline at end of file',
  'diff --git a/docs/new.md b/docs/new.md',
  'new file mode 100644',
  '--- /dev/null',
  '+++ b/docs/new.md',
  '@@ -0,0 +1,2 @@',
  '+# T',
  '+text',
  'diff --git a/old.rs b/old.rs',
  'deleted file mode 100644',
  '--- a/old.rs',
  '+++ /dev/null',
  '@@ -1,2 +0,0 @@',
  '-gone',
  '-gone2',
  'diff --git a/img/a.png b/img/b.png',
  'similarity index 100%',
  'rename from img/a.png',
  'rename to img/b.png',
  'diff --git a/bin.dat b/bin.dat',
  'index 1..2 100644',
  'Binary files a/bin.dat and b/bin.dat differ',
  '',
].join('\n');

test('a diff is parsed into files, hunks and lines with both numbers', () => {
  const files = prParseDiff(SAMPLE);
  assert.deepStrictEqual(plain(files.map(f => [f.path, f.status, f.add, f.del])), [
    ['src/a.rs', 'modified', 3, 2], ['docs/new.md', 'added', 2, 0], ['old.rs', 'deleted', 0, 2], ['img/b.png', 'renamed', 0, 0], ['bin.dat', 'binary', 0, 0]]);
  const a = files[0];
  assert.strictEqual(a.hunks.length, 2);
  assert.deepStrictEqual(plain(a.hunks[0].lines.map(l => [l.kind, l.old, l.new])), [
    ['ctx', 1, 1], ['del', 2, null], ['add', null, 2], ['add', null, 3], ['ctx', 3, 4]]);
  // A missing count is 1, and a hunk header's own numbers start the count.
  assert.deepStrictEqual(plain(a.hunks[1].lines.map(l => [l.kind, l.old, l.new])), [['del', 10, null], ['add', null, 11], ['meta', null, null]]);
  assert.strictEqual(a.hunks[1].oldStart, 10);
  assert.strictEqual(files[3].oldPath, 'img/a.png');
  assert.deepStrictEqual(plain(a.all.map(l => l.k)), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.deepStrictEqual(plain(prParseDiff('')), []);
  assert.deepStrictEqual(plain(prParseDiff(null)), []);
});

test('a removed line that looks like a header, and a quoted path, stay what they are', () => {
  const text = ['diff --git a/x.md b/x.md', '--- a/x.md', '+++ b/x.md', '@@ -1,3 +1,2 @@', ' keep', '--- a/inner', '-diff --git a/z b/z',
    'diff --git "a/caf\\303\\251.txt" "b/caf\\303\\251.txt"', 'new file mode 100644', '--- /dev/null', '+++ "b/caf\\303\\251.txt"', '@@ -0,0 +1 @@', '+hi', ''].join('\n');
  const files = prParseDiff(text);
  assert.strictEqual(files.length, 2);
  assert.deepStrictEqual(plain(files[0].all.map(l => [l.kind, l.text])), [['ctx', 'keep'], ['del', '-- a/inner'], ['del', 'diff --git a/z b/z']]);
  assert.strictEqual(files[1].path, 'café.txt');
  // A path with a space, an empty context line and a carriage return.
  const odd = prParseDiff('diff --git a/a b.txt b/a b.txt\n--- a/a b.txt\n+++ b/a b.txt\n@@ -1,3 +1,3 @@\n one\r\n\n-two\r\n+tw0\r\n');
  assert.strictEqual(odd[0].path, 'a b.txt');
  assert.deepStrictEqual(plain(odd[0].all.map(l => [l.kind, l.text])), [['ctx', 'one'], ['ctx', ''], ['del', 'two'], ['add', 'tw0']]);
  // A file with only a header (a mode change) and one with no `+++` take the path of the `diff --git` line.
  assert.strictEqual(prParseDiff('diff --git a/m.sh b/m.sh\nold mode 100644\nnew mode 100755\n')[0].path, 'm.sh');
});

test('the category of a path: the first rule that matches wins', () => {
  const cases = {
    'src/ui/tests/pr-read.test.js': 'test', 'src/ui/pr-read.js': 'ui', 'src/ui/pr-panel.css': 'ui', 'commands/adj-reviewer.md': 'doc',
    'src/others/tests.rs': 'test', 'tests/others.rs': 'test', 'Cargo.lock': 'resource', 'Cargo.toml': 'logic', 'config.example.json': 'resource',
    'src/others/submit_read.rs': 'logic', 'docs/images/x.png': 'doc', 'src/others/review_tests.rs': 'test',
    'app/src/FooTest.kt': 'test', 'app/src/latest.kt': 'logic', 'app/MainActivity.kt': 'ui', 'app/res/drawable/a.xml': 'resource', 'app/res/layout/a.xml': 'ui',
    'app/res/values/strings.xml': 'resource', '.github/workflows/ci.yml': 'logic', 'web/__snapshots__/a.snap': 'resource', 'dist/app.js': 'resource',
    'a/b.min.js': 'resource', 'README.md': 'doc', 'LICENSE': 'doc', 'icon.svg': 'resource', 'test_x.py': 'test', 'Makefile': 'logic', 'package.json': 'logic',
    'data/rows.csv': 'resource', 'main.go': 'logic', 'src/Button.tsx': 'ui', 'ios/HomeView.swift': 'ui', 'go.sum': 'resource', 'vendor/x/y.go': 'resource',
  };
  for (const [p, cat] of Object.entries(cases)) assert.strictEqual(prDiffCategory(p), cat, p);
  assert.strictEqual(prDiffIsGenerated('Cargo.lock'), true);
  assert.strictEqual(prDiffIsGenerated('src/a.rs'), false);
  assert.strictEqual(prDiffIsGenerated('x/__snapshots__/y.txt'), true);
});

const readOf = (extra = {}) => ({
  sha: 'a'.repeat(40), madeAt: 'x', order: [{ path: 'docs/new.md', category: 'doc' }, { path: 'nope.rs', category: 'logic' }, { path: 'src/a.rs', category: 'logic' }],
  deleted: [{ path: 'old.rs', reason: 'moved', question: null }], annotations: [], ...extra });

test('the layout puts files in reading order, then the rest, with what the reader said of each file', () => {
  const files = prParseDiff(SAMPLE);
  const rec = { files: [{ path: 'src/a.rs', additions: 3, deletions: 2 }, { path: 'missing/x.rs', additions: 4, deletions: 0 }] };
  const layout = prDiffLayout(files, readOf(), rec);
  assert.deepStrictEqual(plain(layout.files.map(f => [f.i, f.path, f.cat])), [[0, 'docs/new.md', 'doc'], [1, 'src/a.rs', 'logic'], [2, 'old.rs', 'logic'],
    [3, 'img/b.png', 'resource'], [4, 'bin.dat', 'logic']]);
  assert.strictEqual(layout.files[2].whole, true);
  assert.strictEqual(layout.files[2].deleted.reason, 'moved');
  assert.strictEqual(layout.files[0].whole, false);
  // `missing/x.rs` is not in the diff: it has no file and is counted once.
  assert.strictEqual(layout.missing, 1);
  assert.ok(!('cats' in layout));
  // Without a word from the reader a file is not skippable and has no note.
  assert.deepStrictEqual(plain(layout.files.map(f => [f.skippable, f.note])), [[false, ''], [false, ''], [false, ''], [false, ''], [false, '']]);
  const said = prDiffLayout(files, readOf({ order: [{ path: 'src/a.rs', category: 'logic', note: 'core', skippable: false }, { path: 'docs/new.md', category: 'doc', note: 'just docs', skippable: true }] }), null);
  assert.deepStrictEqual(plain(said.files.slice(0, 2).map(f => [f.path, f.skippable, f.note])), [['src/a.rs', false, 'core'], ['docs/new.md', true, 'just docs']]);
  assert.strictEqual(layout.rows, files.reduce((n, f) => n + f.all.length + f.hunks.length, 0));
  // Only the diff says a file is deleted whole: the record is of the head now, the diff of the one that was read.
  const l2 = prDiffLayout(files, readOf(), { files: [{ path: 'src/a.rs', change: 'deleted' }] });
  assert.strictEqual(l2.files[1].whole, false);
  assert.deepStrictEqual(plain(prDiffLayout([], null, null)), { files: [], missing: 0, rows: 0 });
});

test('an anchor is found as text, then with its marker, then with whitespace made alike; nth picks, and none is null', () => {
  const [a] = prParseDiff(SAMPLE);
  assert.deepStrictEqual(plain(prDiffAnchor(a, { anchor: 'new();' })), { hunk: 0, line: 3 });
  // Only the marker makes this one: `+    new` is not in the text of a line, but `+` with its text has it.
  assert.deepStrictEqual(plain(prDiffAnchor(a, { anchor: '+    new' })), { hunk: 0, line: 3 });
  // Whitespace made alike: spaces around the anchor, and its inner runs, do not stop a match; a space the line lacks does.
  assert.deepStrictEqual(plain(prDiffAnchor(a, { anchor: '  old();   ' })), { hunk: 0, line: 1 });
  assert.deepStrictEqual(plain(prDiffAnchor(a, { anchor: 'new(  );' })), null);
  assert.deepStrictEqual(plain(prDiffAnchor(a, { anchor: 'fn   a()' })), { hunk: 0, line: 0 });
  const twice = prParseDiff('diff --git a/t b/t\n--- a/t\n+++ b/t\n@@ -1,2 +1,2 @@\n+foo(1)\n+foo(2)\n-foo(0)\n+z\n');
  assert.strictEqual(prDiffAnchor(twice[0], { anchor: 'foo' }).line, 0);
  assert.strictEqual(prDiffAnchor(twice[0], { anchor: 'foo', nth: 2 }).line, 1);
  assert.strictEqual(prDiffAnchor(twice[0], { anchor: 'foo', nth: 3 }).line, 2);
  assert.strictEqual(prDiffAnchor(twice[0], { anchor: 'foo', nth: 4 }), null);
  assert.strictEqual(prDiffAnchor(a, { anchor: 'nowhere' }), null);
  assert.strictEqual(prDiffAnchor(a, { anchor: '' }), null);
  // A `\ No newline` line is never matched.
  assert.strictEqual(prDiffAnchor(a, { anchor: 'No newline' }), null);
});

const one = (p, body, head = '@@ -1,9 +1,9 @@') => prParseDiff(`diff --git a/${p} b/${p}\n--- a/${p}\n+++ b/${p}\n${head}\n${body}\n`)[0];

test('a tr annotation takes the whole comment block, skipping the other side, and an ex takes its line', () => {
  const rs = one('s.rs', ' // one\n-let gone = 1;\n+// two\n+// three\n+let x = 1;\n // tail', '@@ -1,4 +1,5 @@');
  // The match is the added `// two`: the block is every right-side comment line around it, the removed line being skipped.
  assert.deepStrictEqual(plain(prDiffCommentBlock(rs, 2)), { ks: [0, 2, 3], last: 3 });
  assert.deepStrictEqual(plain(prDiffCommentBlock(rs, 4)), { ks: [4], last: 4 });
  // From a removed line, only the left side counts.
  const left = one('s.rs', '-// a\n+// b\n-// c\n+code', '@@ -1,2 +1,2 @@');
  assert.deepStrictEqual(plain(prDiffCommentBlock(left, 0)), { ks: [0, 2], last: 2 });
  const py = one('s.py', '+# a\n+# b\n+x = 1\n+#!/bin/sh');
  assert.deepStrictEqual(plain(prDiffCommentBlock(py, 0)), { ks: [0, 1], last: 1 });
  assert.deepStrictEqual(plain(prDiffCommentBlock(py, 3)), { ks: [3], last: 3 });
  const block = one('b.rs', '+/*\n+ * long\n+ text\n+ */\n+code();');
  assert.deepStrictEqual(plain(prDiffCommentBlock(block, 2)), { ks: [0, 1, 2, 3], last: 3 });
  const html = one('b.html', '+<!--\n+ x\n+-->\n+<p>');
  assert.deepStrictEqual(plain(prDiffCommentBlock(html, 1)), { ks: [0, 1, 2], last: 2 });
  // A closed comment above is not an opener for the code under it.
  const closed = one('c.rs', '+/* a */\n+code();\n+more();');
  assert.deepStrictEqual(plain(prDiffCommentBlock(closed, 2)), { ks: [2], last: 2 });
  // A block does not cross a hunk.
  const hunks = prParseDiff('diff --git a/h.rs b/h.rs\n--- a/h.rs\n+++ b/h.rs\n@@ -1 +1 @@\n+// a\n@@ -9 +9 @@\n+// b\n')[0];
  assert.deepStrictEqual(plain(prDiffCommentBlock(hunks, 0)), { ks: [0], last: 0 });
});

test('annotations are resolved to lines, tints, unanchored ones and orphans', () => {
  const files = prParseDiff(SAMPLE);
  const read = readOf({ annotations: [
    { path: 'src/a.rs', anchor: '// why', kind: 'tr', text: 'T1' },
    { path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'E1' },
    { path: 'src/a.rs', anchor: 'not there', kind: 'ex', text: 'E2' },
    { path: 'gone/file.rs', anchor: 'x', kind: 'tr', text: 'E3' },
  ] });
  const r = prDiffResolve(files, read);
  const a = r.files.get('src/a.rs');
  assert.deepStrictEqual(plain([...a.byLine]), [[2, [0]], [3, [1]]]);
  assert.deepStrictEqual(plain([...a.tint].sort()), [2, 3]);
  assert.deepStrictEqual(plain(a.unanchored), [2]);
  assert.strictEqual(a.count, 3);
  assert.strictEqual(a.at.get(1), 'right:3');
  assert.deepStrictEqual(plain(r.orphans), [3]);
  assert.strictEqual(r.files.get('docs/new.md').count, 0);
});

test('one number column, GitHub sides, and a comment with an empty body', () => {
  const [a] = prParseDiff(SAMPLE);
  const [ctx1, del, add] = a.all;
  assert.deepStrictEqual(plain([prDiffShownNo(ctx1), prDiffShownNo(del), prDiffShownNo(add), prDiffShownNo(a.all[7])]), [1, 2, 2, null]);
  assert.deepStrictEqual(plain([prDiffSideOf(ctx1), prDiffSideOf(del), prDiffSideOf(add)]), ['right', 'left', 'right']);
  const doc = { drafts: [{ key: 'p1' }] };
  assert.deepStrictEqual(plain(prDiffDraft(doc, { path: 'a.rs', line: 4, side: 'left' })), { key: 'p2', source: 'person', path: 'a.rs', line: 4, side: 'left', body: '' });
  assert.deepStrictEqual(plain(prDiffDraft(doc, { path: 'a.rs', body: 'b' })), { key: 'p2', source: 'person', path: 'a.rs', side: 'right', body: 'b' });
  // A concern's draft the person made theirs is the reader's; the head of the read-through on screen is kept.
  assert.deepStrictEqual(plain(prDiffDraft(doc, { path: 'a.rs', line: 2, body: 'b', source: 'reader', sha: 'a'.repeat(40) })), { key: 'p2', source: 'reader', path: 'a.rs', line: 2, side: 'right', body: 'b', sha: 'a'.repeat(40) });
  assert.strictEqual(prDiffDraft(doc, { path: 'a.rs', source: 'other' }).source, 'person');
  assert.deepStrictEqual(plain(prDiffAtOf('left:12')), { side: 'left', line: 12 });
  assert.strictEqual(prDiffAtOf('up:1'), null);
  assert.strictEqual(prDiffAtOf(undefined), null);
});

test('a point\'s line is found on the new side first, then the old, else null', () => {
  const [a] = prParseDiff(SAMPLE);
  assert.strictEqual(prDiffFindLine(a, 3), 3);   // `+    new();` is new line 3; the old line 3 is the closing brace
  assert.strictEqual(prDiffFindLine(a, 11), 6);  // `+y`
  assert.strictEqual(prDiffFindLine(a, 10), 5);  // only the removed `-x` has old number 10 (the new side has 11)
  assert.strictEqual(prDiffFindLine(a, 99), null);
  assert.strictEqual(prDiffFindLine(a, null), null);
});

test('a comment\'s line is found by its side: the right is the added and context lines by new number, the left the removed lines by old number', () => {
  const [a] = prParseDiff(SAMPLE);
  assert.strictEqual(prDiffFindAt(a, 'right', 3), 3);    // `+    new();`
  assert.strictEqual(prDiffFindAt(a, 'right', 4), 4);    // the closing brace, a context line
  assert.strictEqual(prDiffFindAt(a, 'left', 2), 1);     // `-    old();`
  assert.strictEqual(prDiffFindAt(a, 'left', 3), null, 'old line 3 is a context line: the left side has removed lines only');
  assert.strictEqual(prDiffFindAt(a, 'right', 10), null, 'the removed `-x` is old number 10 on the left, not on the right');
  assert.strictEqual(prDiffFindAt(a, 'left', 10), 5);
  assert.strictEqual(prDiffFindAt(a, 'right', 11), 6);
  // The same number on the two sides is two lines; `prDiffFindLine` still takes the right first.
  assert.strictEqual(prDiffFindAt(a, 'right', 2), 2);
  assert.strictEqual(prDiffFindAt(a, 'left', 2), 1);
  assert.strictEqual(prDiffFindLine(a, 2), 2);
  assert.strictEqual(prDiffFindAt(a, 'right', 99), null);
  assert.strictEqual(prDiffFindAt(a, 'right', null), null);
  assert.strictEqual(prDiffFindAt(a, 'right', 0), null);
  // An unknown side is the right.
  assert.strictEqual(prDiffFindAt(a, undefined, 3), 3);
  assert.strictEqual(prDiffFindAt(null, 'right', 3), null);
});

test('a comment is placed on its line, in its file\'s slot, or not at all', () => {
  const files = prParseDiff(SAMPLE);
  const entries = prDiffLayout(files, readOf({}), null).files;
  const at = d => { const p = prDiffPlaceOf(entries, d); return p && { path: p.entry.path, k: p.k, lost: p.lost }; };
  assert.deepStrictEqual(plain(at({ path: 'src/a.rs', line: 3, side: 'right' })), { path: 'src/a.rs', k: 3, lost: false });
  assert.deepStrictEqual(plain(at({ path: 'src/a.rs', line: 2, side: 'left' })), { path: 'src/a.rs', k: 1, lost: false });
  assert.strictEqual(at({ path: 'src/a.rs', line: 2 }).k, 2, 'no side is the right');
  // A comment on the file: no line, so the slot under the header, and nothing is lost.
  assert.deepStrictEqual(plain(at({ path: 'src/a.rs' })), { path: 'src/a.rs', k: null, lost: false });
  // A line the file's diff does not have: the slot, and the header says so.
  assert.deepStrictEqual(plain(at({ path: 'src/a.rs', line: 3, side: 'left' })), { path: 'src/a.rs', k: null, lost: true });
  assert.deepStrictEqual(plain(at({ path: 'src/a.rs', line: 500, side: 'right' })), { path: 'src/a.rs', k: null, lost: true });
  // No path, or a path the diff does not have: not on the diff.
  assert.strictEqual(prDiffPlaceOf(entries, { body: 'x' }), null);
  assert.strictEqual(prDiffPlaceOf(entries, { path: 'nope.rs', line: 1 }), null);
  assert.strictEqual(prDiffPlaceOf(null, { path: 'src/a.rs' }), null);
  // The ones the diff cannot show are told apart: no path, and a path that is not in it (all of them, with no diff; none while it is on its way).
  const drafts = [{ key: 'a', path: 'src/a.rs', line: 3 }, { key: 'b', body: 'whole' }, { key: 'c', path: 'nope.rs' }];
  assert.deepStrictEqual(plain(prDiffUnplaced(entries, drafts)), { whole: [drafts[1]], off: [drafts[2]] });
  assert.deepStrictEqual(plain(prDiffUnplaced(null, drafts)), { whole: [drafts[1]], off: [drafts[0], drafts[2]] });
  assert.deepStrictEqual(plain(prDiffUnplaced(undefined, drafts)), { whole: [drafts[1]], off: [] });
});

test('a comment is drawn with who wrote it and its state; one already sent is read-only, and the head it was written on is said only when it differs', () => {
  const A = 'a'.repeat(40);
  const B = 'b'.repeat(40);
  const d = { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: 'Why <b>?\nBecause.' };
  const saved = prdCommentHtml(d, { readSha: A });
  assert.ok(saved.includes('data-prd-cmt="p1"') && saved.includes('>あなた</span>') && saved.includes('>未送信</span>'));
  assert.ok(saved.includes('Why &lt;b&gt;?\nBecause.') && saved.includes('data-prd-cmt-act="edit"') && saved.includes('data-prd-cmt-act="delete"'));
  assert.ok(!saved.includes('AI の文から') && !saved.includes('の差分で書きました') && !saved.includes('行が見つからない'));
  // From the reader's concern: its chip too.
  assert.ok(prdCommentHtml({ ...d, source: 'reader' }, { readSha: A }).includes('>AI の文から</span>'));
  // The head: no chip without one (files from before), none for the same, a warning for another.
  assert.ok(!prdCommentHtml({ ...d, sha: A }, { readSha: A }).includes('の差分で書きました'));
  const other = prdCommentHtml({ ...d, sha: B }, { readSha: A });
  assert.ok(other.includes('>bbbbbbb の差分で書きました</span>') && other.includes('title="下読みし直す前の差分の行です。行がずれていないか確かめてください"'));
  assert.ok(!prdCommentHtml({ ...d, sha: B }, {}).includes('の差分で書きました'), 'no read-through on screen: nothing to differ from');
  // A line that is not found is said in the header.
  assert.ok(prdCommentHtml(d, { lost: true }).includes('行が見つからないコメント（src/a.rs:3）'));
  // Sent: its own chip, no 編集, and 削除 says what it does not do.
  const sent = prdCommentHtml({ ...d, sentAt: '20261001T000000Z' }, {});
  assert.ok(sent.includes('>積みました</span>') && !sent.includes('>未送信<') && !sent.includes('data-prd-cmt-act="edit"'));
  assert.ok(sent.includes('data-prd-cmt-act="delete" title="adj の一覧から消します（GitHub の pending review や worker への指示は消えません）"'));
  assert.ok(prdCommentHtml({ ...d, sentAt: '20261001T000000Z' }, { to: 'worker' }).includes('>渡しました</span>'));
  // Why a push left it out.
  assert.ok(prdCommentHtml(d, { failedWhy: 'line is not in the diff' }).includes('積めませんでした: line is not in the diff'));
  assert.ok(!prdCommentHtml({ ...d, sentAt: '20261001T000000Z' }, { failedWhy: 'x' }).includes('積めませんでした'));
});

test('an editor is a box with 保存 or コメントを足す, off while it is blank; the text in it is escaped and keeps a first newline', () => {
  const d = { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: 'old' };
  const edit = prdCommentHtml(d, { box: { text: '\n<i>x</i>' } });
  assert.ok(edit.includes('prd-cmt-edit') && edit.includes('<textarea class="prd-cmt-body" data-prd-box="p1"') && edit.includes('placeholder="コメントを書く"'));
  assert.ok(edit.includes('>\n\n&lt;i&gt;x&lt;/i&gt;</textarea>'), edit);
  assert.ok(edit.includes('data-prd-cmt-act="cancel"') && edit.includes('>やめる</button>') && edit.includes('data-prd-cmt-act="save"') && edit.includes('>保存</button>'));
  assert.ok(!/data-prd-cmt-act="save"[^>]* disabled/.test(edit));
  assert.ok(/data-prd-cmt-act="save"[^>]* disabled/.test(prdCommentHtml(d, { box: { text: ' \n' } })));
  // A new one has no draft yet: its key is its place.
  const fresh = prdCommentHtml({ path: 'src/a.rs', line: 3, side: 'left', source: 'reader', body: '' }, { box: { isNew: true, text: '' } });
  assert.ok(fresh.includes('data-prd-cmt="new:src/a.rs|left|3"') && fresh.includes('data-prd-box="new:src/a.rs|left|3"') && fresh.includes('>AI の文から</span>'));
  assert.ok(fresh.includes('>コメントを足す</button>') && /data-prd-cmt-act="add"[^>]* disabled/.test(fresh) && !fresh.includes('data-prd-cmt-act="edit"'));
  assert.strictEqual(prdBoxKey({ path: 'f.rs', side: 'x', line: null }), 'new:f.rs|right|');
});

test('the comments of a diff: the drafts in their order, then the open editors; each with its file and line, and what changes the element and what does not', () => {
  const files = prParseDiff(SAMPLE);
  const entries = prDiffLayout(files, readOf({}), null).files;
  const A = 'a'.repeat(40);
  const doc = { drafts: [
    { key: 'p1', source: 'person', path: 'src/a.rs', line: 3, side: 'right', body: 'one', sha: A },
    { key: 'p2', source: 'person', path: 'src/a.rs', body: 'file' },
    { key: 'p3', source: 'person', path: 'src/a.rs', line: 77, side: 'right', body: 'gone' },
    { key: 'p4', source: 'person', body: 'whole PR' },
    { key: 'p5', source: 'person', path: 'nope.rs', body: 'elsewhere' },
    { key: 'p6', source: 'person', path: 'docs/new.md', line: 1, side: 'right', body: 'sent', sentAt: '20261001T000000Z' },
  ] };
  const boxes = new Map([['p1', { kind: 'edit', text: 'typed' }], ['new:docs/new.md|right|2', { kind: 'new', path: 'docs/new.md', side: 'right', line: 2, source: 'person', text: '' }],
    ['new:old.rs|right|', { kind: 'new', path: 'old.rs', side: 'right', line: null, source: 'person', text: 'x' }], ['new:nope.rs|right|1', { kind: 'new', path: 'nope.rs', side: 'right', line: 1, source: 'person', text: '' }]]);
  const items = prdCommentItems(entries, doc, boxes, { readSha: A, failed: new Map([['p1', 'why']]) });
  const ix = path => entries.find(f => f.path === path).i;
  assert.deepStrictEqual(plain(items.map(x => [x.key, x.kind, x.i, x.k, x.lost])), [
    ['p1', 'edit', ix('src/a.rs'), 3, false], ['p2', 'saved', ix('src/a.rs'), null, false], ['p3', 'saved', ix('src/a.rs'), null, true], ['p6', 'sent', ix('docs/new.md'), 0, false],
    ['new:docs/new.md|right|2', 'new', ix('docs/new.md'), 1, false], ['new:old.rs|right|', 'new', ix('old.rs'), null, false]]);
  // The one being edited shows what is typed; what is not placed (no path, a path the diff lacks) is not here, whether a draft or an editor.
  assert.ok(items[0].html.includes('typed') && !items[0].html.includes('one'));
  // The sig is the element's markup without what is typed: typing changes the html and not the sig, a change of state changes the sig.
  const again = prdCommentItems(entries, doc, new Map([['p1', { kind: 'edit', text: 'more typed' }]]), { readSha: A, failed: new Map([['p1', 'why']]) });
  assert.notStrictEqual(again[0].html, items[0].html);
  assert.strictEqual(again[0].sig, items[0].sig);
  const moved = prdCommentItems(entries, { drafts: [{ ...doc.drafts[0], sha: 'b'.repeat(40) }] }, new Map([['p1', { kind: 'edit', text: 'typed' }]]), { readSha: A });
  assert.notStrictEqual(moved[0].sig, items[0].sig, 'another head is a warning chip');
  const plainOne = prdCommentItems(entries, { drafts: [doc.drafts[0]] }, null, { readSha: A });
  assert.strictEqual(plainOne[0].kind, 'saved');
  assert.notStrictEqual(plainOne[0].sig, items[0].sig);
  // A comment that was sent cannot be edited, whatever box is open for it.
  const sent = prdCommentItems(entries, { drafts: [doc.drafts[5]] }, new Map([['p6', { kind: 'edit', text: 'x' }]]), {});
  assert.strictEqual(sent[0].kind, 'sent');
  // The same text in two comments is two elements; the keys are the draft's and the place's.
  assert.strictEqual(new Set(items.map(x => x.key)).size, items.length);
});

test('the labels of the index are the shortest suffix that is unique', () => {
  const labels = prDiffLabels(['src/a/mod.rs', 'src/b/mod.rs', 'src/x.rs', 'lib/x.rs', 'top.rs', 'deep/er/x.rs']);
  assert.deepStrictEqual(plain([...labels].map(([p, l]) => [p, l.label, l.title])), [
    ['src/a/mod.rs', 'a/mod.rs', 'src/a/mod.rs'], ['src/b/mod.rs', 'b/mod.rs', 'src/b/mod.rs'], ['src/x.rs', 'src/x.rs', 'src/x.rs'],
    ['lib/x.rs', 'lib/x.rs', 'lib/x.rs'], ['top.rs', 'top.rs', 'top.rs'], ['deep/er/x.rs', 'er/x.rs', 'deep/er/x.rs']]);
});

test('everything of the diff and of the reader is escaped in the markup', () => {
  const bad = '<script>alert(1)</script>';
  const text = `diff --git a/${bad} b/${bad}\n--- a/${bad}\n+++ b/${bad}\n@@ -1 +1 @@\n-${bad}\n+${bad} "q"\n`;
  const files = prParseDiff(text);
  const read = readOf({ order: [], annotations: [{ path: files[0].path, anchor: bad, kind: 'ex', text: bad }, { path: files[0].path, anchor: `${bad}x`, kind: 'tr', text: bad }] });
  const layout = prDiffLayout(files, read, null);
  const res = prDiffResolve(files, read);
  const html = prDiffBodyHtml(layout.files[0], res.files.get(files[0].path), read) + prDiffShellHtml(layout.files[0], res.files.get(files[0].path))
    + prDiffBarHtml(layout);
  assert.ok(!html.includes('<script>'), html);
  assert.ok(html.includes('&lt;script&gt;'));
});

test('the bar says only that files of the record are missing from the diff, and is blank when none are', () => {
  const files = prParseDiff(SAMPLE);
  assert.strictEqual(prDiffBarHtml(prDiffLayout(files, readOf(), null)), '');
  assert.strictEqual(prDiffBarHtml(prDiffLayout(files, readOf(), { files: [{ path: 'src/a.rs', additions: 1, deletions: 0 }] })), '');
  const html = prDiffBarHtml(prDiffLayout(files, readOf(), { files: [{ path: 'tests/x.rs', additions: 5, deletions: 0 }, { path: 'tests/y.rs', additions: 1, deletions: 0 }] }));
  assert.ok(html.includes('差分にないファイル 2 件（GitHub で見てください）') && html.includes('role="note"'));
  assert.ok(!html.includes('prd-tab') && !html.includes('data-prd-fold') && !html.includes('<button'));
});

test('the body puts an annotation after the last line of its block, and the buttons carry the side and line', () => {
  const files = prParseDiff(SAMPLE);
  const read = readOf({ annotations: [{ path: 'src/a.rs', anchor: '// why', kind: 'tr', text: 'T1' }, { path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'E1' },
    { path: 'src/a.rs', anchor: 'missing', kind: 'ex', text: 'E2' }] });
  const layout = prDiffLayout(files, read, null);
  const res = prDiffResolve(files, read);
  const e = layout.files.find(f => f.path === 'src/a.rs');
  const html = prDiffBodyHtml(e, res.files.get('src/a.rs'), read);
  const at = s => html.indexOf(s);
  assert.ok(at('行が見つからない注釈') < at('@@ -1,3'), 'unanchored first');
  assert.ok(at('// why') < at('data-prd-ann="0"') && at('data-prd-ann="0"') < at('new();'), 'tr under its own line, before the next');
  assert.ok(at('new();') < at('data-prd-ann="1"'));
  assert.ok(html.includes('data-prd-plus="1:left:2"') && html.includes('data-prd-plus="1:right:3"'));
  assert.ok(html.includes('aria-label="src/a.rs:3 にコメント"') && html.includes('title="この行にコメント"'));
  assert.ok(html.includes('prd-l prd-a prd-tint'));
  assert.ok(html.includes('data-prd-todraft="1" data-prd-at="right:3"'));
  // `tr` has no buttons; the `\ No newline` row has no number and no 「＋」.
  assert.ok(!/prd-ann-tr[^>]*>(?:(?!<\/div>).)*data-prd-todraft/.test(html));
  assert.ok(html.includes('<div class="prd-l prd-m"><span class="prd-no"></span>'));
  // Every line of every hunk is there.
  assert.strictEqual((html.match(/class="prd-l /g) || []).length, 8);
  assert.ok(prDiffBodyHtml(layout.files.find(f => f.path === 'bin.dat'), res.files.get('bin.dat'), read).includes('バイナリファイルです'));
});

test('a shell has no state, a deleted file starts closed with the reason, and the numbers are in the head', () => {
  const files = prParseDiff(SAMPLE);
  const layout = prDiffLayout(files, readOf(), null);
  const gone = prDiffShellHtml(layout.files.find(f => f.path === 'old.rs'), { count: 0 });
  assert.ok(gone.includes('理由: moved') && gone.includes('data-prd-open="2"') && gone.includes('−2 行を表示') && gone.includes(' hidden style="--prd-rows:'));
  const shell = prDiffShellHtml(layout.files[1], { count: 2 });
  assert.ok(shell.includes('data-prd-cat="logic"') && shell.includes('注釈 2') && shell.includes('+3') && !shell.includes('hidden'));
  assert.ok(!/id="|aria-pressed|prd-folded|分前|飛ばしてよい|prd-file-note/.test(shell));
  const rename = prDiffShellHtml(layout.files.find(f => f.path === 'img/b.png'), null);
  assert.ok(rename.includes('img/a.png → img/b.png'));
});

test('a file of more rows than the limit starts closed behind a button, like a file deleted whole', () => {
  const rows = Array.from({ length: 1600 }, (_, i) => `+line ${i}`).join('\n');
  const big = prParseDiff(`diff --git a/big.rs b/big.rs\n--- a/big.rs\n+++ b/big.rs\n@@ -0,0 +1,1600 @@\n${rows}\n`);
  const small = prParseDiff(SAMPLE);
  const layout = prDiffLayout([...big, ...small], null, null);
  const e = layout.files.find(f => f.path === 'big.rs');
  assert.deepStrictEqual([e.rows, e.closed, e.whole], [1601, true, false]);
  assert.strictEqual(layout.files.find(f => f.path === 'src/a.rs').closed, false);
  assert.strictEqual(layout.files.find(f => f.path === 'old.rs').closed, true);
  const shell = prDiffShellHtml(e, null);
  assert.ok(shell.includes('data-prd-open="0"') && shell.includes('1601 行を表示') && shell.includes('行あるため閉じています') && shell.includes(' hidden style='));
});

test('every control the diff draws has a value no other control of its kind has, even with the same line in two files', () => {
  const two = ['a', 'b'].map(n => `diff --git a/${n}.rs b/${n}.rs\n--- a/${n}.rs\n+++ b/${n}.rs\n@@ -4,2 +4,2 @@\n-x\n+y\n ctx\n`).join('');
  const files = prParseDiff(two);
  const read = readOf({ order: [], annotations: [
    { path: 'a.rs', anchor: 'y', kind: 'ex', text: 'A' }, { path: 'b.rs', anchor: 'y', kind: 'ex', text: 'B' }] });
  const layout = prDiffLayout(files, read, null);
  const res = prDiffResolve(files, read);
  const html = layout.files.map(e => prDiffShellHtml(e, res.files.get(e.path)) + prDiffBodyHtml(e, res.files.get(e.path), read)).join('');
  for (const attr of ['plus', 'todraft', 'ask', 'open']) {
    const values = [...html.matchAll(new RegExp(`data-prd-${attr}="([^"]*)"`, 'g'))].map(m => m[1]);
    assert.ok(values.length >= 2 || attr === 'open', attr);
    assert.strictEqual(new Set(values).size, values.length, `${attr}: ${values}`);
    assert.ok(values.every(v => v !== ''), `${attr} has a value`);
  }
  // The same new line (5) in both files is told apart by the file.
  assert.ok(html.includes('data-prd-plus="0:right:5"') && html.includes('data-prd-plus="1:right:5"'));
  assert.deepStrictEqual(plain(prDiffAtOf('1:right:5')), { side: 'right', line: 5 });
});

test('the paths of a rename are taken as they are, with no a/ or b/ stripped', () => {
  const text = ['diff --git a/a/old.js b/a/new.js', 'similarity index 100%', 'rename from a/old.js', 'rename to a/new.js',
    'diff --git a/x b/b/y', 'rename from x', 'rename to b/y', ''].join('\n');
  const files = prParseDiff(text);
  assert.deepStrictEqual(plain(files.map(f => [f.oldPath, f.path, f.status])), [['a/old.js', 'a/new.js', 'renamed'], ['x', 'b/y', 'renamed']]);
});

test('the annotation 「聞く」 is disabled when no reader runs, and 「コメント」 stays on', () => {
  const files = prParseDiff(SAMPLE);
  const read = readOf({ annotations: [{ path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'E1' }] });
  const layout = prDiffLayout(files, read, null);
  const res = prDiffResolve(files, read);
  const e = layout.files.find(f => f.path === 'src/a.rs');
  const on = prDiffBodyHtml(e, res.files.get('src/a.rs'), read, true);
  assert.ok(/data-prd-ask="0"[^>]*>聞く/.test(on) && !/data-prd-ask="0"[^>]*disabled/.test(on));
  const off = prDiffBodyHtml(e, res.files.get('src/a.rs'), read, false);
  assert.match(off, /data-prd-ask="0"[^>]* disabled title="worker が動いていません">聞く/);
  assert.ok(!/data-prd-todraft="0"[^>]*disabled/.test(off));
});

test('an excerpt is the lines of a range on one side, with what lies between, and nothing for a range the file does not have', () => {
  const [a] = prParseDiff(SAMPLE);
  const excerpt = r => ctx.prDiffExcerpt(a, r);
  // Right side 2..3 is the two added lines; the removed line before them is not between them.
  assert.deepEqual(plain(excerpt({ from: 2, to: 3 }).ks.map(k => a.all[k].text)), ['    // why', '    new();']);
  // From the context line to the added one, the removed line between is taken.
  assert.deepEqual(plain(excerpt({ from: 1, to: 2 }).ks.map(k => a.all[k].kind)), ['ctx', 'del', 'add']);
  // Left side: context and removed lines by their old numbers.
  assert.deepEqual(plain(excerpt({ from: 1, to: 2, side: 'left' }).ks.map(k => a.all[k].kind)), ['ctx', 'del']);
  // Across two hunks: both are named.
  assert.equal(excerpt({ from: 2, to: 11 }).hunks.size, 2);
  assert.equal(excerpt({ from: 500, to: 600 }), null);
  assert.equal(ctx.prDiffExcerpt(null, { from: 1, to: 2 }), null);
});

test('an excerpt is drawn as the diff draws its rows, without the buttons, with a header per hunk and the annotations under their lines', () => {
  const files = prParseDiff(SAMPLE);
  const read = { annotations: [{ path: 'src/a.rs', kind: 'ex', anchor: 'new();', text: 'Why <new>' }, { path: 'src/a.rs', kind: 'tr', anchor: '// why', text: 'because' }] };
  const res = prDiffResolve(files, read).files.get('src/a.rs');
  const ks = ctx.prDiffExcerpt(files[0], { from: 1, to: 11 }).ks;
  const html = ctx.prDiffExcerptHtml(files[0], ks, res, read);
  assert.equal(html.split('class="prd-h"').length - 1, 2);
  assert.ok(html.includes('prd-a') && html.includes('prd-d') && html.includes('prd-tint'));
  assert.ok(html.includes('Why &lt;new&gt;') && html.includes('because'));
  assert.ok(!html.includes('prd-plus') && !html.includes('data-prd-todraft') && !html.includes('data-prd-ask'));
  // Without a resolution there is only the code.
  assert.ok(!ctx.prDiffExcerptHtml(files[0], ks, null, null).includes('prd-ann'));
  // The diff's own annotation keeps its buttons.
  assert.ok(ctx.prdAnnHtml(0, { kind: 'ex', text: 't' }, 'right:1').includes('data-prd-todraft'));
  assert.ok(!ctx.prdAnnHtml(0, { kind: 'ex', text: 't' }, 'right:1', true, { plain: true }).includes('prd-ann-acts'));
});

test('an excerpt with a huge removal between two added lines keeps the lines asked for, is cut to a fixed number of rows and says how many were left out', () => {
  const del = Array.from({ length: 5000 }, (_, i) => `-gone ${i}`);
  const text = ['diff --git a/x b/x', '--- a/x', '+++ b/x', '@@ -1,5001 +1,2 @@', '+first', ...del, '+last'].join('\n');
  const [f] = prParseDiff(text);
  const got = ctx.prDiffExcerpt(f, { from: 1, to: 2 });
  assert.equal(got.ks.length, 80);
  assert.ok(got.ks.includes(0) && got.ks.includes(f.all.length - 1), 'both lines asked for are there');
  const html = ctx.prDiffExcerptHtml(f, got.ks, null, null);
  assert.equal(html.split('class="prd-l ').length - 1, 80 + 1);
  assert.ok(html.includes('… 4922 行省略'));
  // A range with no gap is not marked.
  assert.ok(!ctx.prDiffExcerptHtml(f, [0, 1, 2], null, null).includes('行省略'));
});

test('an excerpt over a span of hundreds of thousands of lines is built from the picks and the start of each gap, not from the span', () => {
  const all = [{ kind: 'add', new: 1, old: null, text: 'a', h: 0, k: 0 }];
  for (let i = 1; i <= 300000; i++) all.push({ kind: 'del', new: null, old: i, text: 'd', h: 0, k: i });
  all.push({ kind: 'add', new: 2, old: null, text: 'b', h: 0, k: 300001 });
  const got = ctx.prDiffExcerpt({ all, hunks: [{ header: '@@' }] }, { from: 1, to: 2 });
  assert.equal(got.ks.length, 80);
  assert.deepEqual([got.ks[0], got.ks[1], got.ks[78], got.ks[79]], [0, 1, 78, 300001]);
});

test('a file the reader called skippable says so in its header, and its note is under the header', () => {
  const files = prParseDiff(SAMPLE);
  const layout = prDiffLayout(files, readOf({ order: [{ path: 'src/a.rs', category: 'logic', note: 'the <core> change', skippable: false }, { path: 'docs/new.md', category: 'doc', note: '   ', skippable: true }] }), null);
  const a = prDiffShellHtml(layout.files.find(f => f.path === 'src/a.rs'), { count: 0 });
  assert.ok(a.includes('<div class="prd-file-note">the &lt;core&gt; change</div>') && !a.includes('飛ばしてよい'));
  assert.ok(a.indexOf('</header>') < a.indexOf('prd-file-note') && a.indexOf('prd-file-note') < a.indexOf('prd-body'));
  const doc = prDiffShellHtml(layout.files.find(f => f.path === 'docs/new.md'), { count: 0 });
  const head = doc.slice(doc.indexOf('<header'), doc.indexOf('</header>'));
  assert.ok(head.includes('<span class="pr-chip pr-skippable">飛ばしてよい</span>') && head.indexOf('ドキュメント') < head.indexOf('飛ばしてよい'));
  // A blank note draws nothing.
  assert.ok(!doc.includes('prd-file-note'));
  // The note of a closed file stays before the line that says it is closed.
  const closed = prDiffLayout(files, readOf({ order: [{ path: 'old.rs', category: 'logic', note: 'gone', skippable: true }] }), null);
  const gone = prDiffShellHtml(closed.files.find(f => f.path === 'old.rs'), { count: 0 });
  assert.ok(gone.indexOf('prd-file-note') < gone.indexOf('prd-gone'));
});

test('an annotation is 訳, 解説 or 気になる; 解説 has 「コメント」, 気になる has 「コメントにする」, both have 「聞く」, 訳 has none, and an unknown kind is 解説', () => {
  const html = (kind, extra = {}, readerOpen, opts) => prdAnnHtml(4, { kind, text: 'T <b>', ...extra }, 'right:3', readerOpen, opts);
  const tr = html('tr');
  assert.ok(tr.includes('prd-ann-tr') && tr.includes('>訳</span>') && !tr.includes('<button'));
  const ex = html('ex');
  assert.ok(ex.includes('prd-ann-ex') && ex.includes('>解説</span>') && ex.includes('data-prd-todraft="4" data-prd-at="right:3">コメント</button>') && ex.includes('data-prd-ask="4"'));
  const concern = html('concern', { draft: 'D' });
  assert.ok(concern.includes('class="prd-ann prd-ann-concern" data-prd-ann="4"') && concern.includes('>気になる</span>'));
  assert.ok(/data-prd-todraft="4" data-prd-at="right:3"[^>]*>コメントにする<\/button>/.test(concern) && concern.includes('data-prd-ask="4"') && concern.includes('T &lt;b&gt;'));
  // The draft is the author's comment: it is not shown in the line, only used by 「コメントにする」.
  assert.ok(!concern.includes('>D<'));
  // The reader's window is gone: 「聞く」 is off with the reason; the comment buttons stay.
  const off = html('concern', {}, false);
  assert.ok(/data-prd-ask="4"[^>]* disabled title="worker が動いていません"/.test(off) && !/data-prd-todraft[^>]*disabled/.test(off));
  // A place that only reads has no buttons, whatever the kind.
  for (const kind of ['ex', 'concern', 'tr']) assert.ok(!html(kind, {}, true, { plain: true }).includes('<button'));
  const odd = html('what');
  assert.ok(odd.includes('prd-ann-ex') && odd.includes('>解説</span>') && odd.includes('data-prd-todraft'));
  assert.ok(html(undefined).includes('prd-ann-ex'));
  // Whole, never folded: the text and the buttons are always in the markup.
  assert.ok(!/prd-folded|hidden/.test(concern));
});

test('a concern goes under its line like an explanation, tints it, and shows plain in an excerpt', () => {
  const files = prParseDiff(SAMPLE);
  const read = readOf({ annotations: [{ path: 'src/a.rs', anchor: 'new();', kind: 'concern', text: 'Why?', draft: 'Could you say why?' }] });
  const res = prDiffResolve(files, read);
  const r = res.files.get('src/a.rs');
  assert.deepStrictEqual(plain([...r.byLine]), [[3, [0]]]);
  assert.strictEqual(r.at.get(0), 'right:3');
  const e = prDiffLayout(files, read, null).files.find(f => f.path === 'src/a.rs');
  const html = prDiffBodyHtml(e, r, read);
  assert.ok(html.includes('prd-ann-concern') && html.indexOf('new();') < html.indexOf('data-prd-ann="0"') && html.includes('prd-l prd-a prd-tint'));
  const excerpt = prDiffExcerptHtml(files[0], [1, 2, 3], r, read);
  assert.ok(excerpt.includes('prd-ann-concern') && excerpt.includes('Why?') && !excerpt.includes('<button') && !excerpt.includes('Could you say why?'));
});

test('the comments of your last review are placed on their lines, without being counted, and the ones not found are left out', () => {
  const files = prParseDiff(SAMPLE);
  const since = { commit: 'b'.repeat(40), newChanges: [], comments: [
    { commentId: 1, path: 'src/a.rs', line: 3, status: 'untouched', note: 'still <here>' },
    { commentId: 2, path: 'src/a.rs', line: 99, status: 'addressed', note: 'no such line' },
    { commentId: 3, path: 'nope.rs', line: 1, status: 'replied', note: 'no such file' },
    { commentId: 4, path: 'src/a.rs', status: 'replied', note: 'no line' },
    { commentId: 5, path: 'src/a.rs', line: 3, status: 'addressed', note: 'second' }] };
  const read = readOf({ annotations: [{ path: 'src/a.rs', anchor: 'new();', kind: 'ex', text: 'E1' }], sinceReview: since });
  const res = prDiffResolve(files, read);
  const r = res.files.get('src/a.rs');
  assert.deepStrictEqual(plain([...r.since]), [[3, [0, 4]]]);
  assert.strictEqual(r.count, 1, 'not counted');
  assert.deepStrictEqual(plain(res.orphans), []);
  assert.deepStrictEqual(plain([...res.files.get('docs/new.md').since]), []);
  assert.deepStrictEqual(plain([...prDiffResolve(files, readOf()).files.get('src/a.rs').since]), []);
  const e = prDiffLayout(files, read, null).files.find(f => f.path === 'src/a.rs');
  const html = prDiffBodyHtml(e, r, read);
  assert.strictEqual((html.match(/prd-ann-prev/g) || []).length, 2);
  assert.ok(html.includes('<div class="prd-ann prd-ann-prev"><span class="prd-ann-k">前回のあなたのコメント <span class="pr-chip pr-since-untouched">手つかず</span></span><div class="prd-ann-text">still &lt;here&gt;</div></div>'));
  assert.ok(html.includes('pr-since-addressed">対応済み</span>') && !html.includes('no such line') && !html.includes('no line'));
  // After the line\'s own annotation.
  assert.ok(html.indexOf('data-prd-ann="0"') < html.indexOf('prd-ann-prev'));
  assert.ok(html.indexOf('new();') < html.indexOf('prd-ann-prev'));
});
