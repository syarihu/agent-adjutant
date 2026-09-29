# Vendored xterm.js

The board terminal (`adj server`, resident boards only) renders a tmux session with
[xterm.js](https://github.com/xtermjs/xterm.js). The files here are embedded in the binary with
`include_str!` and served at `/b/<slug>/vendor/xterm.js` and `/b/<slug>/vendor/xterm.css`
(see `src/cmd/serve.rs`). Nothing is fetched from a CDN at run time.

| Package | Version | Files used | Tarball | sha256 of the tarball |
| --- | --- | --- | --- | --- |
| `@xterm/xterm` | 5.5.0 | `lib/xterm.js`, `css/xterm.css` | https://registry.npmjs.org/@xterm/xterm/-/xterm-5.5.0.tgz | `bd954fa721872170188cc5d7e83e88db3c83c9a18a4e8d24c2783d26491f59d2` |
| `@xterm/addon-fit` | 0.10.0 | `lib/addon-fit.js` | https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.10.0.tgz | `917ac44972453d5eed52edc1e50260c76398ce48cf2290c2e60671102bba0b33` |
| `@xterm/addon-unicode11` | 0.8.0 | `lib/addon-unicode11.js` | https://registry.npmjs.org/@xterm/addon-unicode11/-/addon-unicode11-0.8.0.tgz | `0202c50aaa686eceb13301875788f6465d35bd0d3304bf26a913a0278e434d0f` |

The addon versions are the newest whose `peerDependencies` accept `@xterm/xterm` 5.x.

The files are the UMD builds from the tarballs, byte for byte, except that the final
`//# sourceMappingURL=...` line is removed (the `.map` files are not vendored). Each UMD build
defines a global (`Terminal`, `FitAddon`, `Unicode11Addon`), so the server serves the three
scripts concatenated as one `xterm.js`, prefixed with the license notice.

`LICENSE`, `LICENSE.addon-fit` and `LICENSE.addon-unicode11` are the `LICENSE` files of the
respective packages, verbatim. The MIT notice is also emitted at the top of the served script.

## Updating

1. Pick the versions (the addons must accept the chosen `@xterm/xterm` in `peerDependencies`):
   `curl -s https://registry.npmjs.org/@xterm/xterm | jq '.["dist-tags"]'`.
2. Download and check each tarball:
   `curl -sLO https://registry.npmjs.org/@xterm/xterm/-/xterm-<version>.tgz && shasum -a 256 xterm-<version>.tgz`.
3. Extract it and copy `package/lib/<name>.js` (dropping the `sourceMappingURL` line),
   `package/css/xterm.css` and `package/LICENSE` over the files here.
4. Update the versions and hashes above, then run `cargo test` and open a terminal from a board.
