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
const { prParseDiff, prDiffCategory, prDiffIsGenerated, prDiffLayout, prDiffAnchor, prDiffCommentBlock, prDiffResolve, prDiffDraft, prDiffSideOf,
  prDiffShownNo, prDiffFindLine, prDiffBarHtml, prDiffIndexHtml, prDiffShellHtml, prDiffBodyHtml, prDiffLabels, prDiffAtOf } = ctx;

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

test('the layout puts files in reading order, then the rest, and the shares add up to 100', () => {
  const files = prParseDiff(SAMPLE);
  const rec = { files: [{ path: 'src/a.rs', additions: 3, deletions: 2 }, { path: 'missing/x.rs', additions: 4, deletions: 0 }] };
  const layout = prDiffLayout(files, readOf(), rec);
  assert.deepStrictEqual(plain(layout.files.map(f => [f.i, f.path, f.cat])), [[0, 'docs/new.md', 'doc'], [1, 'src/a.rs', 'logic'], [2, 'old.rs', 'logic'],
    [3, 'img/b.png', 'resource'], [4, 'bin.dat', 'logic']]);
  assert.strictEqual(layout.files[2].whole, true);
  assert.strictEqual(layout.files[2].deleted.reason, 'moved');
  assert.strictEqual(layout.files[0].whole, false);
  // `missing/x.rs` is not in the diff: it counts in the numbers of its tab and has no file.
  assert.deepStrictEqual(plain(layout.cats.logic), { count: 4, drawn: 3, lines: 5 + 2 + 4, share: layout.cats.logic.share });
  assert.strictEqual(layout.missing, 1);
  const shares = Object.values(layout.cats).map(c => c.share);
  assert.strictEqual(shares.reduce((a, b) => a + b, 0), 100);
  assert.strictEqual(layout.rows, files.reduce((n, f) => n + f.all.length + f.hunks.length, 0));
  // Only the diff says a file is deleted whole: the record is of the head now, the diff of the one that was read.
  const l2 = prDiffLayout(files, readOf(), { files: [{ path: 'src/a.rs', change: 'deleted' }] });
  assert.strictEqual(l2.files[1].whole, false);
  assert.deepStrictEqual(Object.values(prDiffLayout([], null, null).cats).map(c => c.share), [0, 0, 0, 0, 0]);
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

test('one number column, GitHub sides, and a draft with an empty body', () => {
  const [a] = prParseDiff(SAMPLE);
  const [ctx1, del, add] = a.all;
  assert.deepStrictEqual(plain([prDiffShownNo(ctx1), prDiffShownNo(del), prDiffShownNo(add), prDiffShownNo(a.all[7])]), [1, 2, 2, null]);
  assert.deepStrictEqual(plain([prDiffSideOf(ctx1), prDiffSideOf(del), prDiffSideOf(add)]), ['right', 'left', 'right']);
  const doc = { drafts: [{ key: 'p1' }] };
  assert.deepStrictEqual(plain(prDiffDraft(doc, { path: 'a.rs', line: 4, side: 'left' })), { key: 'p2', source: 'person', path: 'a.rs', line: 4, side: 'left', body: '' });
  assert.deepStrictEqual(plain(prDiffDraft(doc, { path: 'a.rs', body: 'b' })), { key: 'p2', source: 'person', path: 'a.rs', side: 'right', body: 'b' });
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
    + prDiffIndexHtml(layout, res) + prDiffBarHtml(layout);
  assert.ok(!html.includes('<script>'), html);
  assert.ok(html.includes('&lt;script&gt;'));
});

test('the bar has five toggle buttons in a group, a disabled one for an empty category, and no state in it', () => {
  const layout = prDiffLayout(prParseDiff(SAMPLE), readOf(), null);
  const html = prDiffBarHtml(layout);
  assert.strictEqual((html.match(/<button type="button" class="prd-tab"/g) || []).length, 5);
  assert.ok(html.includes('role="group"'));
  assert.ok(html.includes('data-prd-tab="test" aria-pressed="false" disabled'));
  assert.ok(!html.includes('prd-missing'));
  assert.ok(html.includes('data-prd-fold aria-pressed="false"'));
  assert.ok(/仕様・ロジック<\/span><span class="prd-tab-n">3 · \d+%/.test(html), html);
  assert.ok(!/id="/.test(html) && !/分前/.test(html));
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
  assert.ok(html.includes('aria-label="src/a.rs:3 に下書き"'));
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
  assert.ok(!/id="|aria-pressed|prd-folded|分前/.test(shell));
  const rename = prDiffShellHtml(layout.files.find(f => f.path === 'img/b.png'), null);
  assert.ok(rename.includes('img/a.png → img/b.png'));
});

test('a tab with no file to draw is disabled, and files the diff lacks are said once in the bar', () => {
  const files = prParseDiff(SAMPLE);
  // The record has a test file the diff does not: the tab counts it, but there is nothing to show in it.
  const layout = prDiffLayout(files, readOf(), { files: [{ path: 'tests/x.rs', additions: 5, deletions: 0 }, { path: 'tests/y.rs', additions: 1, deletions: 0 }] });
  assert.strictEqual(layout.cats.test.count, 2);
  assert.strictEqual(layout.cats.test.drawn, 0);
  const html = prDiffBarHtml(layout);
  assert.ok(html.includes('data-prd-tab="test" aria-pressed="false" disabled'));
  assert.ok(html.includes('差分にないファイル 2 件（GitHub で見てください）'));
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
  const html = layout.files.map(e => prDiffShellHtml(e, res.files.get(e.path)) + prDiffBodyHtml(e, res.files.get(e.path), read)).join('') + prDiffIndexHtml(layout, res);
  for (const attr of ['plus', 'todraft', 'ask', 'open', 'jump']) {
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
