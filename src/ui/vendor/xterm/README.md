# Vendored xterm.js

The board terminal (`adj server`, resident boards only) renders a tmux session with
[xterm.js](https://github.com/xtermjs/xterm.js). The files here are embedded in the binary with
`include_str!` and served at `/b/<slug>/vendor/xterm.js` and `/b/<slug>/vendor/xterm.css`
(see `src/transport/board_http/assets.rs`). Nothing is fetched from a CDN at run time.

| Package | Version | Files used | Tarball | sha256 of the tarball |
| --- | --- | --- | --- | --- |
| `@xterm/xterm` | 6.0.0 | `lib/xterm.js`, `css/xterm.css` | https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz | `908e66e04af6c8dc6b00dd3b54de088e2e81e5ed866284fd6c2fb3c2d1c7a3f6` |
| `@xterm/addon-fit` | 0.11.0 | `lib/addon-fit.js` | https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz | `26003b4517a132b64e4ff228fd88a5fda3fff5e606c76093f6dcff772e9ecec0` |
| `@xterm/addon-unicode11` | 0.9.0 | `lib/addon-unicode11.js` | https://registry.npmjs.org/@xterm/addon-unicode11/-/addon-unicode11-0.9.0.tgz | `b665667792f916873fe946ba98cc98a1a742af08e163fb66e1c0809c120ca42e` |

Each is the `latest` dist-tag on the npm registry. The addons declare no `peerDependencies`
at these versions; they are the releases published alongside `@xterm/xterm` 6.0.0.

The files are the UMD builds from the tarballs, byte for byte, except that the final
`//# sourceMappingURL=...` line is removed (the `.map` files are not vendored). Each UMD build
defines a global (`Terminal`, `FitAddon`, `Unicode11Addon`), so the server serves the three
scripts concatenated as one `xterm.js`, prefixed with the license notice.

`LICENSE`, `LICENSE.addon-fit` and `LICENSE.addon-unicode11` are the `LICENSE` files of the
respective packages, verbatim. The MIT notice is also emitted at the top of the served script.

## Updating

1. Pick the versions (the addons must be releases that go with the chosen `@xterm/xterm`):
   `curl -s https://registry.npmjs.org/@xterm/xterm | jq '.["dist-tags"]'`.
2. Download and check each tarball:
   `curl -sLO https://registry.npmjs.org/@xterm/xterm/-/xterm-<version>.tgz && shasum -a 256 xterm-<version>.tgz`.
3. Extract it and copy `package/lib/<name>.js` (dropping the `sourceMappingURL` line),
   `package/css/xterm.css` and `package/LICENSE` over the files here.
4. Update the versions and hashes above, then run `cargo test` and open a terminal from a board.
