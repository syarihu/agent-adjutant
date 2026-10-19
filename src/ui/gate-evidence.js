/* ── A gate's words and evidence, as plain lines and a few lines of its diff ─────────────────────────────────────
   Pure helpers the question card (question-card.js), the TODO (todo.js) and the terminal's question card (work-term.js) share:
   the first lines of a text as one short plain line, and the lines of a gate's diff that back what it asks. They read what the
   server says and return text or markup, touching neither the page nor the network, so `src/ui/tests/gate-evidence.test.js`
   runs them under `node --test`. Loaded after util.js (`esc`), pr-read.js and pr-diff.js (`prParseDiff`, `prDiffResolve`,
   `prDiffExcerptHtml`) and md.js (`mdPlain`), and before question-card.js. */

const GATE_PLAIN_MAX = 140;
/* The first lines of a text as one short plain line: `lines` of them, cut at `max` characters. */
function gatePlainLine(text, lines = 2, max = GATE_PLAIN_MAX) {
  const t = mdPlain(text).split('\n').map(l => l.trim()).filter(Boolean).slice(0, lines).join(' ');
  return [...t].length > max ? `${[...t].slice(0, max).join('')}…` : t;
}

/* The first line of what a gate asks, for a list of waits. */
const gateFocusLine = focus => gatePlainLine(focus, 1, 100);

/* A gate's diff parsed once, and its annotations anchored once for a read-through, not at every redraw. */
/* The gate diffs parsed recently, by their text: a few, so that two parts with different diffs do not parse both at every redraw. */
const GATE_PARSED_KEPT = 4;
const gateParsed = new Map();
function gateParsedDiff(diff) {
  let files = gateParsed.get(diff);
  if (!files) files = prParseDiff(diff).filter(f => f.hunks.length);
  // Taken out and put back last: the one used longest ago is the one let go.
  gateParsed.delete(diff);
  gateParsed.set(diff, files);
  if (gateParsed.size > GATE_PARSED_KEPT) gateParsed.delete(gateParsed.keys().next().value);
  return files;
}
const GATE_RESOLVED = new WeakMap();
function gateResolved(files, read) {
  let byRead = GATE_RESOLVED.get(files);
  if (!byRead) GATE_RESOLVED.set(files, byRead = new WeakMap());
  if (!byRead.has(read)) byRead.set(read, prDiffResolve(files, read));
  return byRead.get(read);
}

/* A few lines of a gate's diff as evidence for what it asks: the first hunk of each file a finding names (`location` is
   `path:line`), else the first hunk of the diff. At most `max` lines in all, drawn as the diff view draws them (without its
   buttons). With the PR's read-through `read` the reader's annotations are under their lines, anchored in this diff.
   `{ html, annotated }`: whether an annotation was drawn is what the card says in its heading. */
function gateEvidenceParts(diff, findings, read = null, max = 12) {
  const files = gateParsedDiff(diff);
  if (!files.length) return { html: '', annotated: false };
  const named = (findings || []).map(f => String(f?.location ?? '').replace(/:\d+(?:-\d+)?$/, '')).filter(Boolean);
  const picked = files.filter(f => named.some(n => n === f.path || n.endsWith(`/${f.path}`) || f.path.endsWith(`/${n}`)));
  const res = read?.annotations?.length ? gateResolved(files, read) : null;
  let left = max;
  let annotated = false;
  const html = [];
  for (const f of picked.length ? picked : files.slice(0, 1)) {
    if (left <= 0) break;
    const ks = f.hunks[0].lines.map(l => l.k).slice(0, left);
    left -= ks.length;
    const r = res?.files.get(f.path) || null;
    if (r && ks.some(k => r.byLine.has(k))) annotated = true;
    html.push(`<div class="qc-ev-file"><code class="pr-loc">${esc(f.path)}</code></div>${prDiffExcerptHtml(f, ks, r, read)}`);
  }
  return { html: html.join(''), annotated };
}
