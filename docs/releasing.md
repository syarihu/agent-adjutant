# Releasing

How to cut a release of agent-adjutant. This page is for the maintainer. The Homebrew formula lives in a
separate repository, `syarihu/homebrew-tap`, and is edited by hand after each release: the Release
workflow builds the binaries, but nothing updates the formula.

## Steps

1. Bump `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` follows. Commit it as
   "Bump version to X.Y.Z" and merge to `main`.
2. Tag the merge commit on `main` as `vX.Y.Z` and push the tag. Wait for the Release workflow
   (`.github/workflows/release.yml`) to finish. Check that the release has the four tarballs
   (`agent-adjutant-{aarch64,x86_64}-apple-darwin.tar.gz` and
   `agent-adjutant-{aarch64,x86_64}-unknown-linux-gnu.tar.gz`) and `checksums.txt`.
3. In `syarihu/homebrew-tap`, update `Formula/agent-adjutant.rb`. Change the version in the four URLs and
   in the two `assert_match "adjutant X.Y.Z"` lines of the test. Change the four `sha256` values using
   `checksums.txt`: each tarball name maps to one `on_macos` / `on_linux` branch (`aarch64` is the
   `Hardware::CPU.arm?` branch, `x86_64` is the `else` branch). To fetch the checksums:

   ```bash
   gh release download vX.Y.Z -R syarihu/agent-adjutant -p checksums.txt -O -
   ```

   For the first release that contains #789, also add the `depends_on` lines and the `caveats` in
   [The formula's dependencies and caveats](#the-formulas-dependencies-and-caveats).

4. Keep `depends_on` and `caveats` in step with the install steps in `README.md` and `README.ja.md`.
5. Check the result. Run `brew update && brew upgrade syarihu/tap/agent-adjutant` (or a fresh
   `brew install syarihu/tap/agent-adjutant`) and `brew test syarihu/tap/agent-adjutant`. Then run the
   steps in the caveats once.
6. Edit the generated release notes when the release changes a default or needs an action from users.
   For the release that makes tmux the default terminal preset (#789), say that iTerm2 now needs
   `"terminal": {"preset": "iterm2"}` and that a running server needs `adj server restart`.

## The formula's dependencies and caveats

The first release that contains #789 needs these lines in the formula, and they stay afterwards. Put
`depends_on` after `license` and before `on_macos` (Homebrew's component order, with `gh` before `tmux`):

```ruby
  depends_on "gh"
  depends_on "tmux"
```

Put `caveats` after `install` and before `test`:

```ruby
  def caveats
    <<~EOS
      To get started:
        gh auth login
        adjutant install-mcp          # registers the MCP server with Claude Code
                                      # (--target agy for Antigravity, --target json for others)
        adj server start              # starts the board (http://127.0.0.1:4577 by default)
        cd <repository> && adj hub    # the repository appears on the board

      After brew upgrade, run adj server restart so the server runs the new version.
    EOS
  end
```

Why it looks like this:

- tmux and gh come with the formula via `depends_on`, because from the release that makes tmux the default
  terminal preset (#789) the board's terminal and the workers need tmux, and the hub and workers read issues
  and open PRs through `gh`.
- There is no `service` block, because the server is started with `adj server start`.
- The formula does not run `adjutant install-mcp`, because it changes the agent's settings. It is only a step in the caveats.

`adj server start` only starts the board. A check for what is missing before start is tracked in #790.
When it ships, the caveats and the README may say so.
