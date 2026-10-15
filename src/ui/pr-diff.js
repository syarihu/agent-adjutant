/* ── The annotated diff of a PR waiting on your review: parsing, grouping, anchoring and markup ─────────────────────────
   Pure, like pr-read.js: the text of `GET /api/others/diff` and the read-through of `GET /api/others/read` go in, plain values
   and strings come out, with no page and no storage, so `src/ui/tests/pr-diff.test.js` runs them under `node --test`. The
   page's half (what is fetched, filled, shown and clicked) is pr-diff-view.js. Loaded after pr-read.js.

   Everything that came from the diff or from the reader goes through `esc`, and nothing of it is Markdown. No markup here
   carries an `id`, a time, a draft, or which tab or fold is on: those are set in place by the page, so a redraw of the panel or
   of 「拡大」 (which compares markup) does not empty the files that were filled. */

const PRD_CATS = ['logic', 'ui', 'doc', 'test', 'resource'];
/* A file with more rows than this starts closed behind a button, as a file deleted whole does: drawing it is one write to the page. */
const PRD_BIG_ROWS = 1500;
const PRD_CAT_TAB = { logic: '仕様・ロジック', ui: 'UI', doc: 'ドキュメント', test: 'テスト', resource: 'リソース' };

/* ── parsing ── */

/* A path git quoted because of what is in it: `"a\303\251.txt"` is `aé.txt`. */
function prdUnquote(s) {
  if (!(s.length >= 2 && s[0] === '"' && s[s.length - 1] === '"')) return s;
  const cs = [...s.slice(1, -1)];
  const named = { n: 10, t: 9, a: 7, b: 8, f: 12, r: 13, v: 11, '\\': 92, '"': 34 };
  const bytes = [];
  const enc = new TextEncoder();
  for (let i = 0; i < cs.length; i++) {
    if (cs[i] !== '\\' || i + 1 >= cs.length) { bytes.push(...enc.encode(cs[i])); continue; }
    const next = cs[++i];
    if (/[0-7]/.test(next)) {
      let oct = next;
      while (oct.length < 3 && /[0-7]/.test(cs[i + 1] || '')) oct += cs[++i];
      bytes.push(parseInt(oct, 8) & 255);
    } else if (next in named) bytes.push(named[next]);
    else bytes.push(...enc.encode(next));
  }
  return new TextDecoder().decode(Uint8Array.from(bytes));
}
/* The paths of `rename from` / `rename to` have no `a/` or `b/` in front, so a rename to `a/new.js` is that path. */
/* A path of a `---` / `+++` line, or of the `diff --git` line: unquoted, without git's `a/` or `b/` and the tab it adds. */
const prdPathOf = raw => {
  const p = prdUnquote(String(raw).replace(/\t.*$/, '').trim());
  return p === '/dev/null' ? null : p.replace(/^[ab]\//, '');
};
function prdHeaderPaths(rest) {
  const word = '("(?:[^"\\\\]|\\\\.)*"|\\S+)';
  const m = rest.match(new RegExp(`^${word} ${word}$`));
  if (m) return [prdPathOf(m[1]), prdPathOf(m[2])];
  // A path with a space in it: the same path on both sides, else split at the last ` b/`.
  const same = rest.match(/^a\/(.+) b\/\1$/);
  if (same) return [same[1], same[1]];
  const at = rest.lastIndexOf(' b/');
  return at > 0 ? [prdPathOf(rest.slice(0, at)), prdPathOf(rest.slice(at + 1))] : [null, null];
}

const PRD_HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/* The hunk that starts at `lines[at]`: exactly as many old and new lines as its header says are taken, so a content line that
   looks like `diff --git` or `--- a/x` stays content. Returns where the next line to read is. */
function prdHunk(f, lines, at) {
  const m = lines[at].match(PRD_HUNK_RE);
  let oldN = Number(m[1]);
  let newN = Number(m[3]);
  let oldLeft = m[2] === undefined ? 1 : Number(m[2]);
  let newLeft = m[4] === undefined ? 1 : Number(m[4]);
  const h = f.hunks.length;
  const hunk = { header: lines[at].replace(/\r$/, ''), oldStart: oldN, newStart: newN, lines: [] };
  f.hunks.push(hunk);
  const push = (kind, text, old, nu) => {
    const l = { kind, old, new: nu, text: text.replace(/\r$/, ''), h, k: f.all.length };
    hunk.lines.push(l);
    f.all.push(l);
  };
  let i = at + 1;
  while (i < lines.length) {
    const s = lines[i];
    // `\ No newline at end of file` can follow the last line of a hunk, after the counts ran out.
    if (s.startsWith('\\')) { push('meta', s, null, null); i++; continue; }
    if (oldLeft <= 0 && newLeft <= 0) break;
    const c = s[0];
    if (c === '+') {
      if (newLeft <= 0) break;
      push('add', s.slice(1), null, newN++); newLeft--;
    } else if (c === '-') {
      if (oldLeft <= 0) break;
      push('del', s.slice(1), oldN++, null); oldLeft--;
    } else if (c === ' ' || s === '') {
      // An empty line is a context line whose space an editor took off.
      if (oldLeft <= 0 || newLeft <= 0) break;
      push('ctx', s.slice(1), oldN++, newN++); oldLeft--; newLeft--;
    } else break;
    i++;
  }
  return i;
}

/* The files of a unified diff, in the order it has them:
   `{ path, oldPath, status: added|deleted|renamed|modified|binary, add, del, hunks: [{ header, oldStart, newStart, lines }], all }`.
   A line is `{ kind: add|del|ctx|meta, old, new, text, h (its hunk), k (its place among all the file's lines) }`. */
function prParseDiff(text) {
  const src = String(text ?? '');
  if (!src) return [];
  const lines = src.split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  const files = [];
  let f = null;
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith('diff --git ')) {
      const [a, b] = prdHeaderPaths(line.slice('diff --git '.length).replace(/\r$/, ''));
      f = { path: b ?? a ?? '', oldPath: a, status: 'modified', add: 0, del: 0, hunks: [], all: [], via: { header: true } };
      files.push(f);
      i++;
    } else if (!f) i++;
    else if (line.startsWith('@@') && PRD_HUNK_RE.test(line)) i = prdHunk(f, lines, i);
    else {
      // The header lines of a file; after the last hunk anything else (a mail signature) is not part of the diff.
      if (!f.hunks.length) {
        if (line.startsWith('new file mode')) f.status = 'added';
        else if (line.startsWith('deleted file mode')) f.status = 'deleted';
        else if (line.startsWith('rename from ')) { f.rename = true; f.renameFrom = prdUnquote(line.slice(12).replace(/\r$/, '')); }
        else if (line.startsWith('rename to ')) f.renameTo = prdUnquote(line.slice(10).replace(/\r$/, ''));
        else if (line.startsWith('--- ')) f.minus = prdPathOf(line.slice(4));
        else if (line.startsWith('+++ ')) f.plus = prdPathOf(line.slice(4));
        else if (/^Binary files .* differ$/.test(line) || line.startsWith('GIT binary patch')) f.binary = true;
      }
      i++;
    }
  }
  for (const x of files) {
    // `+++`, else `---` (a deleted file), else the rename, else the `diff --git` line (a binary file has no other).
    const named = x.plus ?? x.minus ?? x.renameTo;
    if (named) x.path = named;
    if (x.rename) { x.oldPath = x.renameFrom ?? x.oldPath; x.status = 'renamed'; }
    if (x.binary) x.status = 'binary';
    for (const l of x.all) { if (l.kind === 'add') x.add++; else if (l.kind === 'del') x.del++; }
    delete x.via; delete x.minus; delete x.plus; delete x.rename; delete x.renameFrom; delete x.renameTo; delete x.binary;
  }
  return files;
}

/* ── the category of a path ── */

const prdParts = path => {
  const parts = String(path || '').split('/').filter(Boolean);
  const base = parts.pop() || '';
  return { dirs: parts.map(s => s.toLowerCase()), base, lower: base.toLowerCase(), ext: (base.includes('.') ? base.split('.').pop() : '').toLowerCase() };
};
const PRD_LOCKS = ['cargo.lock', 'package-lock.json', 'yarn.lock', 'pnpm-lock.yaml', 'gemfile.lock', 'poetry.lock', 'composer.lock', 'go.sum'];

/* Lock files, snapshots, minified and generated files, vendored code: what nobody reads by hand. (A later issue's "not counted"
   rule is this one.) commands/adj-reviewer.md ("Split proposal") repeats these lists for the reader: keep them in step. */
function prDiffIsGenerated(path) {
  const { dirs, lower } = prdParts(path);
  if (PRD_LOCKS.includes(lower) || lower.endsWith('.lock')) return true;
  if (/\.(snap|map|min\.js|min\.css|pb\.go|g\.dart)$/.test(lower) || /_pb2\.py$/.test(lower) || /\.generated\./.test(lower)) return true;
  return dirs.some(d => ['__snapshots__', 'vendor', 'node_modules', 'dist'].includes(d));
}

/* One of logic, ui, doc, test, resource, from the path alone. The first rule that matches wins, in this order: generated, test,
   doc, media, ui, build and CI, data and config, else logic. A suffix such as `FooTest.kt` is matched with its capital T, so
   `latest.kt` is not a test. */
function prDiffCategory(path) {
  const { dirs, base, lower, ext } = prdParts(path);
  const inDir = (...names) => dirs.some(d => names.includes(d));
  const full = [...dirs, lower].join('/');
  if (prDiffIsGenerated(path)) return 'resource';
  if (inDir('test', 'tests', '__tests__', 'spec', 'specs', 'testdata', 'fixtures', 'androidtest', 'testfixtures', 'e2e')
    || /\.(test|spec)\./.test(lower) || /_test\./.test(lower) || /^test_.*\.py$/.test(lower) || /_spec\.rb$/.test(lower)
    || /[A-Za-z0-9](Test|Tests)\.(kt|java|swift|scala|groovy|cs)$/.test(base) || /[A-Za-z0-9]Tests\.[^.]+$/.test(base)
    || lower === 'conftest.py' || lower === 'tests.rs' || /_tests\.rs$/.test(lower)) return 'test';
  if (['md', 'mdx', 'markdown', 'rst', 'adoc', 'txt'].includes(ext) || inDir('docs', 'doc', 'documentation')
    || /^(readme|changelog|license|contributing)/.test(lower) || lower === 'authors' || lower === 'notice') return 'doc';
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'ico', 'bmp', 'ttf', 'otf', 'woff', 'woff2', 'mp3', 'mp4', 'wav', 'pdf'].includes(ext)
    || /(^|\/)res\/(drawable|mipmap|raw|font)[^/]*\//.test(full)) return 'resource';
  if (['css', 'scss', 'sass', 'less', 'styl', 'html', 'htm', 'vue', 'svelte', 'jsx', 'tsx', 'xib', 'storyboard', 'ejs', 'hbs', 'erb', 'njk', 'twig'].includes(ext)
    || inDir('ui', 'components', 'views', 'screens', 'pages', 'layouts', 'templates', 'widgets') || /(^|\/)res\/layout[^/]*\//.test(full)
    || /(View|ViewController)\.swift$/.test(base) || /(Screen|Activity|Fragment)\.kt$/.test(base)) return 'ui';
  if (['cargo.toml', 'package.json', 'build.gradle', 'build.gradle.kts', 'pom.xml', 'go.mod', 'pyproject.toml', 'setup.py', 'gemfile', 'makefile', 'dockerfile'].includes(lower)
    || /^settings\.gradle/.test(lower) || ext === 'mk' || inDir('.github')) return 'logic';
  if (['json', 'yaml', 'yml', 'toml', 'ini', 'xml', 'properties', 'csv', 'strings', 'po', 'arb'].includes(ext)
    || /(^|\/)res\/values[^/]*\//.test(full) || inDir('assets', 'resources', 'i18n', 'locales', 'l10n')) return 'resource';
  return 'logic';
}

/* ── the layout: files in reading order, and the share of each category ── */

/* Whole numbers that add up to 100 (the largest remainders get the leftover), all zero when there is nothing. */
function prdShares(lines) {
  const total = lines.reduce((a, b) => a + b, 0);
  if (!total) return lines.map(() => 0);
  const raw = lines.map(n => (n * 100) / total);
  const out = raw.map(Math.floor);
  const order = raw.map((r, i) => [r - out[i], i]).sort((a, b) => b[0] - a[0] || a[1] - b[1]);
  for (let left = 100 - out.reduce((a, b) => a + b, 0), j = 0; left > 0; left--, j++) out[order[j % order.length][1]]++;
  return out;
}

/* `{ files: [{ i, path, file, cat, add, del, rows, whole, closed, deleted }], cats: { logic: { count, drawn, lines, share }, ... }, missing, rows }`.
   The files are in the reader's reading order (those of `read.order` that the diff has), then the others in the diff's own order. `whole` is
   a file deleted whole (by the diff's own word: the record is of the PR's head now, the diff of the read's), `deleted` what the reader said of it (`read.deleted`), `closed`
   that it starts closed (deleted whole, or more than `PRD_BIG_ROWS` rows). A file of `rec.files` that the diff does not have counts
   in the tabs' numbers only (`count`), not in `drawn`, and is counted in `missing`. */
function prDiffLayout(files, read, rec) {
  const byPath = new Map(files.map(f => [f.path, f]));
  const seen = new Set();
  const ordered = [];
  for (const o of read?.order || []) {
    if (byPath.has(o.path) && !seen.has(o.path)) { seen.add(o.path); ordered.push(byPath.get(o.path)); }
  }
  for (const f of files) if (!seen.has(f.path)) { seen.add(f.path); ordered.push(f); }
  const deletedBy = new Map((read?.deleted || []).map(d => [d.path, d]));
  const entries = ordered.map((f, i) => ({
    i, path: f.path, file: f, cat: prDiffCategory(f.path), add: f.add, del: f.del, rows: f.all.length + f.hunks.length,
    whole: f.status === 'deleted', deleted: deletedBy.get(f.path) || null,
  }));
  entries.forEach(e => { e.closed = e.whole || e.rows > PRD_BIG_ROWS; });
  const cats = Object.fromEntries(PRD_CATS.map(c => [c, { count: 0, drawn: 0, lines: 0, share: 0 }]));
  for (const e of entries) { cats[e.cat].count++; cats[e.cat].drawn++; cats[e.cat].lines += e.add + e.del; }
  let missing = 0;
  for (const x of rec?.files || []) {
    if (byPath.has(x.path)) continue;
    missing++;
    const c = cats[prDiffCategory(x.path)];
    c.count++; c.lines += (Number(x.additions) || 0) + (Number(x.deletions) || 0);
  }
  prdShares(PRD_CATS.map(c => cats[c].lines)).forEach((s, i) => { cats[PRD_CATS[i]].share = s; });
  return { files: entries, cats, missing, rows: entries.reduce((n, e) => n + e.rows, 0) };
}

/* ── anchoring the reader's annotations ── */

/* The line an annotation is about: `{ hunk, line }` (`line` is the line's place among the file's, `k`), or null. The anchor is a
   substring of one diff line. It is looked for in the text of the lines first; failing that with the marker (`+`, `-`, space)
   in front, and failing that with whitespace made alike. `nth` (default 1) picks among the lines the first pass that found any
   matched; more than there are is null. */
function prDiffAnchor(file, ann) {
  const anchor = String(ann?.anchor ?? '');
  if (!anchor) return null;
  const nth = Math.max(1, Number(ann.nth) || 1);
  const norm = s => s.replace(/\s+/g, ' ').trim();
  const marker = l => (l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' ');
  const passes = [
    l => l.text.includes(anchor),
    l => (marker(l) + l.text).includes(anchor),
    norm(anchor) ? l => norm(l.text).includes(norm(anchor)) : null,
  ].filter(Boolean);
  for (const hit of passes) {
    const found = file.all.filter(l => l.kind !== 'meta' && hit(l));
    if (found.length) return found[nth - 1] ? { hunk: found[nth - 1].h, line: found[nth - 1].k } : null;
  }
  return null;
}

/* How a file's comments start, by its extension. An unknown one takes every family. */
function prdCommentFamilies(path) {
  const { lower, ext } = prdParts(path);
  const c = ['rs', 'js', 'mjs', 'cjs', 'ts', 'jsx', 'tsx', 'java', 'kt', 'kts', 'swift', 'go', 'c', 'h', 'cpp', 'hpp', 'cc', 'cs', 'scala', 'dart', 'php', 'css', 'scss', 'groovy', 'gradle'];
  const hash = ['py', 'rb', 'sh', 'bash', 'zsh', 'yaml', 'yml', 'toml', 'mk', 'r', 'pl', 'nix', 'ex', 'exs', 'tf'];
  if (c.includes(ext)) return ['c'];
  if (hash.includes(ext) || lower === 'makefile' || lower === 'dockerfile') return ['hash'];
  if (['sql', 'lua', 'hs'].includes(ext)) return ['dash'];
  if (['html', 'htm', 'xml', 'vue', 'svelte', 'md', 'mdx'].includes(ext)) return ['markup'];
  return ['c', 'hash', 'dash', 'markup'];
}
const PRD_COMMENT_START = {
  c: t => t.startsWith('//') || t.startsWith('/*') || t === '*' || t.startsWith('* ') || t.startsWith('*/'),
  hash: t => t.startsWith('#') && !/^#(!|\[|include\b)/.test(t),
  dash: t => t === '--' || t.startsWith('-- '),
  markup: t => t.startsWith('<!--') || t.startsWith('-->'),
};
const PRD_BLOCKS = { c: ['/*', '*/'], markup: ['<!--', '-->'] };

/* The lines of the comment block that line `k` belongs to, and the one the annotation goes under (`last`): `{ ks, last }`.
   A line that is not a comment is a block of itself. The block runs up and down over lines of the same side (a right-side line
   skips removed lines, a left-side one skips added lines), within the hunk, and stops at the first that is not a comment. A line
   between the opener and the closer of a C block comment or an HTML comment (at most 200 lines of its side apart) is in the block
   from one to the other. */
function prDiffCommentBlock(file, k, path = file.path) {
  const all = file.all;
  const at = all[k];
  if (!at || at.kind === 'meta') return { ks: [k], last: k };
  const side = at.kind === 'del' ? 'left' : 'right';
  const same = l => (side === 'left' ? l.kind === 'del' : l.kind === 'add' || l.kind === 'ctx');
  const families = prdCommentFamilies(path);
  const inHunk = i => i >= 0 && i < all.length && all[i].h === at.h;
  const sameSide = (from, step) => {
    const out = [];
    for (let i = from; inHunk(i); i += step) if (same(all[i])) out.push(i);
    return out;
  };
  for (const fam of families.filter(f => PRD_BLOCKS[f])) {
    const [open, close] = PRD_BLOCKS[fam];
    const up = sameSide(k, -1).slice(0, 200);
    const o = (() => {
      for (const i of up) {
        const t = all[i].text;
        if (i < k && t.includes(close)) return -1;
        if (t.includes(open) && (!t.includes(close) || t.lastIndexOf(open) > t.lastIndexOf(close))) return i;
      }
      return -1;
    })();
    if (o < 0) continue;
    const c = sameSide(k, 1).slice(0, 200).find(i => i > o && all[i].text.includes(close));
    if (c === undefined) continue;
    const ks = sameSide(o, 1).filter(i => i <= c);
    return { ks, last: c };
  }
  const isComment = i => {
    const t = all[i].text.trim();
    return families.some(f => PRD_COMMENT_START[f](t));
  };
  if (!isComment(k)) return { ks: [k], last: k };
  const ks = [k];
  for (const i of sameSide(k - 1, -1)) { if (!isComment(i)) break; ks.unshift(i); }
  for (const i of sameSide(k + 1, 1)) { if (!isComment(i)) break; ks.push(i); }
  return { ks, last: ks[ks.length - 1] };
}

/* Where each annotation goes: `{ files: Map(path -> { byLine: Map(k -> [annotation index]), tint: Set(k), unanchored: [index], at: Map(index -> 'right:42'), count }), orphans: [index] }`.
   A `tr` goes under the last line of its comment block and tints the block; an `ex` goes under its line and tints it. An
   annotation whose anchor is not found is `unanchored` in its file; one whose path is not in the diff is an orphan. */
function prDiffResolve(files, read) {
  const out = new Map(files.map(f => [f.path, { byLine: new Map(), tint: new Set(), unanchored: [], at: new Map(), count: 0 }]));
  const orphans = [];
  (read?.annotations || []).forEach((ann, idx) => {
    const file = files.find(f => f.path === ann.path);
    if (!file) { orphans.push(idx); return; }
    const r = out.get(file.path);
    r.count++;
    const found = prDiffAnchor(file, ann);
    if (!found) { r.unanchored.push(idx); return; }
    const block = ann.kind === 'tr' ? prDiffCommentBlock(file, found.line) : { ks: [found.line], last: found.line };
    block.ks.forEach(k => r.tint.add(k));
    if (!r.byLine.has(block.last)) r.byLine.set(block.last, []);
    r.byLine.get(block.last).push(idx);
    const l = file.all[found.line];
    r.at.set(idx, `${prDiffSideOf(l)}:${prDiffShownNo(l)}`);
  });
  return { files: out, orphans };
}

/* ── lines and drafts ── */

/* GitHub's numbering: a removed line is on the left at its old number, every other line on the right at its new one. */
function prDiffSideOf(l) { return l?.kind === 'del' ? 'left' : 'right'; }
/* The one number column: the new number, and for a removed line the old one. */
function prDiffShownNo(l) { return l?.kind === 'del' ? l.old : l?.kind === 'meta' ? null : l?.new ?? null; }
/* The place (`k`) of the line a point's `line` names: the added or context line with that new number, else the removed line
   with that old number, else null (the file's top). */
function prDiffFindLine(file, line) {
  const n = Number(line);
  if (!n) return null;
  const hit = file.all.find(l => (l.kind === 'add' || l.kind === 'ctx') && l.new === n) || file.all.find(l => l.kind === 'del' && l.old === n);
  return hit ? hit.k : null;
}
/* A draft of the person's own, from a line of the diff (or a file, with no `line`): the body may be empty. */
function prDiffDraft(doc, { path, line = null, side = 'right', body = '' }) {
  const d = { key: prNextKey(doc, 'p'), source: 'person', path };
  if (line) d.line = line;
  d.side = side === 'left' ? 'left' : 'right';
  d.body = body;
  return d;
}
/* "right:42" (or "3:right:42", with the file's place in front) as `{ side, line }`, or null. */
function prDiffAtOf(text) {
  const m = String(text || '').match(/^(?:\d+:)?(right|left):(\d+)$/);
  return m ? { side: m[1], line: Number(m[2]) } : null;
}

/* ── markup ── */

/* The tabs (real buttons in a group: `aria-pressed` and which are shown are set in place by the page) and the fold button, whose
   label stays the same (its state is `aria-pressed`). A category with no file to draw is disabled and still drawn, so the bar looks
   the same in every tab; files of the record that the diff lacks are said once. */
function prDiffBarHtml(layout) {
  const tabs = PRD_CATS.map(c => {
    const s = layout.cats[c];
    return `<button type="button" class="prd-tab" data-prd-tab="${c}" aria-pressed="false"${s.drawn ? '' : ' disabled'}><span class="prd-tab-name">${esc(PRD_CAT_TAB[c])}</span><span class="prd-tab-n">${s.count} · ${s.share}%</span></button>`;
  }).join('');
  return `<div class="prd-bar"><div class="prd-tabs" role="group" aria-label="差分のカテゴリ">${tabs}</div>`
    + '<button type="button" class="btn-m3-text prd-fold" data-prd-fold aria-pressed="false">注釈をたたむ</button></div>'
    + (layout.missing ? `<div class="tp-muted prd-missing" role="note">${esc(`差分にないファイル ${layout.missing} 件（GitHub で見てください）`)}</div>` : '');
}

const prdFileName = e => (e.file.status === 'renamed' && e.file.oldPath ? `${e.file.oldPath} → ${e.path}` : e.path);
const prdStat = e => `<span class="prd-stat"><span class="add">+${e.add}</span> <span class="del">−${e.del}</span></span>`;

/* A file's header and an empty body to be filled later (`--prd-rows` holds its height). A file deleted whole, or a very long one,
   starts closed behind a button (a deleted one with what the reader said of it). */
function prDiffShellHtml(e, res) {
  const n = res?.count || 0;
  const why = e.whole
    ? (e.deleted ? (e.deleted.reason ? `理由: ${esc(e.deleted.reason)}` : `作者への質問: ${esc(e.deleted.question)}`) : '丸ごと削除されたファイルです')
    : `${e.rows} 行あるため閉じています`;
  const gone = e.closed
    ? `<div class="prd-gone">${why} <button type="button" class="btn-m3-text prd-open" data-prd-open="${e.i}">${e.whole ? `−${e.del} 行を表示` : `${e.rows} 行を表示`}</button></div>` : '';
  return `<section class="m3-filled-card pr-sec prd-file" data-prd-file="${e.i}" data-prd-cat="${e.cat}">`
    + `<header class="prd-file-head"><code class="pr-loc prd-path">${esc(prdFileName(e))}</code><span class="pr-chip">${esc(PRD_CAT_TAB[e.cat])}</span>${prdStat(e)}${n ? `<span class="tp-muted">注釈 ${n}</span>` : ''}</header>`
    + `${gone}<div class="prd-body" data-prd-body="${e.i}"${e.closed ? ' hidden' : ''} style="--prd-rows:${e.rows}"></div></section>`;
}

const PRD_ANN_LABEL = { tr: '訳', ex: '解説' };
/* `opts.plain` leaves out the buttons, for a place that only reads (the aide's display area); the default is the diff's own. */
function prdAnnHtml(idx, ann, at, readerOpen = true, opts = {}) {
  const kind = ann.kind === 'tr' ? 'tr' : 'ex';
  const acts = kind === 'ex' && !opts.plain
    ? `<div class="prd-ann-acts"><button type="button" class="btn-m3-text pr-act" data-prd-todraft="${idx}"${at ? ` data-prd-at="${esc(at)}"` : ''}>下書きに</button><button type="button" class="btn-m3-text pr-act" data-prd-ask="${idx}"${at ? ` data-prd-at="${esc(at)}"` : ''}${readerOpen ? '' : ` disabled title="${esc(PR_ASK_OFF_WHY)}"`}>聞く</button></div>` : '';
  return `<div class="prd-ann prd-ann-${kind}" data-prd-ann="${idx}"><span class="prd-ann-k">${PRD_ANN_LABEL[kind]}</span><div class="prd-ann-text">${esc(ann.text)}</div>${acts}</div>`;
}

/* A file's hunks, every line of them, with the annotations under their lines. `readerOpen` false turns the annotations' 「聞く」 off. */
function prDiffBodyHtml(e, res, read, readerOpen = true) {
  const anns = read?.annotations || [];
  const f = e.file;
  const lost = (res?.unanchored || []).map(idx => `<div class="prd-ann prd-ann-lost"><span class="prd-ann-k">行が見つからない注釈</span> <code>${esc(anns[idx].anchor)}</code>`
    + `<div class="prd-ann-text">${esc(anns[idx].text)}</div></div>`).join('');
  if (f.status === 'binary') return `${lost}<div class="tp-muted prd-note">バイナリファイルです</div>`;
  if (!f.hunks.length) return `${lost}<div class="tp-muted prd-note">内容の変更はありません</div>`;
  const cls = { add: 'prd-a', del: 'prd-d', ctx: 'prd-c' };
  const rows = f.hunks.map(h => `<div class="prd-h">${esc(h.header)}</div>` + h.lines.map(l => {
    if (l.kind === 'meta') return `<div class="prd-l prd-m"><span class="prd-no"></span><span class="prd-sign"></span><span class="prd-code">${esc(l.text)}</span></div>`;
    const no = prDiffShownNo(l);
    const side = prDiffSideOf(l);
    const label = `${e.path}:${no}${side === 'left' ? '（削除された側）' : ''} に下書き`;
    const line = `<div class="prd-l ${cls[l.kind]}${res?.tint.has(l.k) ? ' prd-tint' : ''}" data-prd-i="${l.k}"><span class="prd-no">${no}</span>`
      + `<span class="prd-sign">${l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '}</span><span class="prd-code">${esc(l.text)}</span>`
      + `<button type="button" class="prd-plus" data-prd-plus="${e.i}:${side}:${no}" aria-label="${esc(label)}" title="この行に下書き">＋</button></div>`;
    return line + (res?.byLine.get(l.k) || []).map(idx => prdAnnHtml(idx, anns[idx], res.at.get(idx), readerOpen)).join('');
  }).join('')).join('');
  return `${lost}${rows}`;
}

/* The label of each path in 「拡大」's index: the shortest suffix of its path that no other file of the diff shares (the file name
   unless two files have it), with the whole path as the title. */
function prDiffLabels(paths) {
  const segs = paths.map(p => String(p).split('/').filter(Boolean));
  // How many paths have each suffix, by its length: a path is told apart at the first length no other path shares.
  const count = new Map();
  segs.forEach(s => { for (let n = 1; n <= s.length; n++) { const k = `${n}\0${s.slice(-n).join('/')}`; count.set(k, (count.get(k) || 0) + 1); } });
  return new Map(paths.map((p, idx) => {
    const s = segs[idx];
    const suffix = n => s.slice(-n).join('/');
    let n = 1;
    while (n < s.length && count.get(`${n}\0${suffix(n)}`) > 1) n++;
    return [p, { label: suffix(n) || p, title: p }];
  }));
}

/* ── a few lines of a file ── */

/* The lines numbered `from` to `to` on `side` (right: the added and context lines by their new number, left: the removed and
   context lines by their old one), and every line between the first and the last of them, removed ones among them:
   `{ ks, hunks }` (`ks` are the lines' places, `hunks` the hunks they are in), or null when no line of the file is in the range. */
function prDiffExcerpt(file, { from, to, side = 'right' }) {
  const left = side === 'left';
  const no = l => (left ? (l.kind === 'del' || l.kind === 'ctx' ? l.old : null) : (l.kind === 'add' || l.kind === 'ctx' ? l.new : null));
  const picked = (file?.all || []).filter(l => l.kind !== 'meta' && no(l) != null && no(l) >= from && no(l) <= to);
  if (!picked.length) return null;
  const ks = [];
  for (let k = picked[0].k; k <= picked[picked.length - 1].k; k++) ks.push(k);
  return { ks, hunks: new Set(ks.map(k => file.all[k].h)) };
}

/* The lines `ks` of `file` as the diff draws them, without the buttons: a hunk's header again wherever the lines pass into another
   hunk, the annotations of `res` (the file's part of `prDiffResolve`, with `read`) under their lines. */
function prDiffExcerptHtml(file, ks, res, read) {
  const anns = read?.annotations || [];
  const cls = { add: 'prd-a', del: 'prd-d', ctx: 'prd-c' };
  let hunk = null;
  return ks.map(k => {
    const l = file.all[k];
    if (!l) return '';
    const head = l.h !== hunk ? `<div class="prd-h">${esc(file.hunks[l.h]?.header || '')}</div>` : '';
    hunk = l.h;
    if (l.kind === 'meta') return `${head}<div class="prd-l prd-m"><span class="prd-no"></span><span class="prd-sign"></span><span class="prd-code">${esc(l.text)}</span></div>`;
    const line = `<div class="prd-l ${cls[l.kind]}${res?.tint.has(l.k) ? ' prd-tint' : ''}" data-prd-i="${l.k}"><span class="prd-no">${prDiffShownNo(l)}</span>`
      + `<span class="prd-sign">${l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '}</span><span class="prd-code">${esc(l.text)}</span></div>`;
    const under = (res?.byLine.get(l.k) || []).map(idx => prdAnnHtml(idx, anns[idx], res.at.get(idx), false, { plain: true })).join('');
    return head + line + under;
  }).join('');
}
