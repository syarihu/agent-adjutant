/* ── The HTML report a gate carries ──────────────────────────────────────────────────────────────────────────────────────────────
   A worker's report is a page it wrote: it can draw what it likes, and it runs in a frame the board does not trust. The frame is
   sandboxed without the same-origin token, so the report has an opaque origin and reaches neither the page, its token nor its storage,
   and its policy lets it fetch nothing. Loaded after dialog-stack.js, whose layers it pushes. */

/* The policy the report runs under; the same string the server sends with it (REPORT_POLICY in src/transport/board_http/report.rs, and
   a test holds the two together). */
const REPORT_CSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'";

/* The report with its policy as the first thing the parser sees. A frame loaded from a blob takes the policy of the page that made it,
   not the server's header, so the policy has to travel inside the document; and a policy in a meta only covers what comes after it, so
   nothing of the report may come before. It goes right after a doctype that only whitespace comes before, for a tag in front of it
   would put the report in quirks mode, and at the very start otherwise. Not after a comment: the parser ends one at the first `-->`,
   or at once for `<!-->`, so a pattern that let comments through could be led past a script to a doctype further down. */
function reportDocument(html) {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${esc(REPORT_CSP)}">`;
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  const at = doctype ? doctype[0].length : 0;
  return html.slice(0, at) + meta + html.slice(at);
}

/* Show the report of gate `g` in a layer of its own. The layer opens at once and the report is read after: it goes to the frame as a
   blob rather than as `srcdoc`, for the address of a `srcdoc` frame is the page's, token included. It is fetched with the token in a
   header, which is the only way the server hands it out. */
function openGateReport(g, opener, title) {
  const key = `html-report:${gateRef(g)}`;
  let url = null;
  let closed = false;
  const release = () => {
    closed = true;
    if (url) URL.revokeObjectURL(url);
    url = null;
  };
  const pushed = dialogStack.push({
    key, opener, title: `レポート — ${title}`, sub: esc(g.report || ''), cls: 'qd-report', onClose: release,
    body: '<div class="qc-report"><div class="qc-report-h"><span class="material-symbols-outlined" aria-hidden="true">lock</span>worker が書いた HTML（自己完結 · sandbox で表示）</div>'
      + '<iframe class="qc-report-frame" sandbox="allow-scripts" referrerpolicy="no-referrer" title="レポート"></iframe><p class="qc-report-err" hidden></p></div>',
  });
  if (!pushed) return false;
  const layer = () => (closed ? null : dialogStack.layerEl(key));
  const fail = why => {
    const el = layer();
    if (!el?.isConnected) return;
    el.querySelector('.qc-report-frame').hidden = true;
    const err = el.querySelector('.qc-report-err');
    err.textContent = `レポートを読めませんでした: ${why}`;
    err.hidden = false;
  };
  (async () => {
    try {
      const res = await fetch(`${baseOf(g)}/api/gates/${encodeURIComponent(g.id)}/report`, { headers: { 'X-Adjutant-Token': TOKEN } });
      if (!res.ok) {
        const why = await res.json().then(j => j?.error, () => null);
        return fail(why || `HTTP ${res.status}`);
      }
      const text = await res.text();
      const el = layer();
      // Closed while it was being read: nothing is left to show it in.
      if (!el?.isConnected) return;
      // UTF-8 said in the type: the text is UTF-8 here, and a report without a charset meta would otherwise be read in the browser's default.
      url = URL.createObjectURL(new Blob([reportDocument(text)], { type: 'text/html;charset=utf-8' }));
      el.querySelector('.qc-report-frame').src = url;
    } catch (e) {
      fail(e?.message || String(e));
    }
  })();
  return true;
}
