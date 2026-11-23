# Releasing

How to cut a release of agent-adjutant. This page is for the maintainer. The Homebrew formula lives in a
separate repository, `syarihu/homebrew-tap`, and is edited by hand after each release: the Release
workflow builds the binaries, but nothing updates the formula.

## Steps

1. Bump `version` in `Cargo.toml` and in `app/src-tauri/Cargo.toml` (the macOS app ships in the same
   tarball; `make check` fails while the two differ). Run `cargo build` at the root and in
   `app/src-tauri` so both `Cargo.lock` files follow. Commit it as "Bump version to X.Y.Z" and merge to
   `main`.
2. Tag the merge commit on `main` as `vX.Y.Z` and push the tag. Wait for the Release workflow
   (`.github/workflows/release.yml`) to finish. Check that the release has the four tarballs
   (`agent-adjutant-{aarch64,x86_64}-apple-darwin.tar.gz` and
   `agent-adjutant-{aarch64,x86_64}-unknown-linux-gnu.tar.gz`) and `checksums.txt`. The two macOS
   tarballs also hold `Adjutant.app`.
3. In `syarihu/homebrew-tap`, update `Formula/agent-adjutant.rb`. Change the version in the four URLs and
   in the two `assert_match "adjutant X.Y.Z"` lines of the test. Change the four `sha256` values using
   `checksums.txt`: each tarball name maps to one `on_macos` / `on_linux` branch (`aarch64` is the
   `Hardware::CPU.arm?` branch, `x86_64` is the `else` branch). To fetch the checksums:

   ```bash
   gh release download vX.Y.Z -R syarihu/agent-adjutant -p checksums.txt -O -
   ```

   For the first release whose macOS tarballs hold `Adjutant.app`, also make the changes in
   [The macOS app in the formula](#the-macos-app-in-the-formula).
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

## The macOS app in the formula

From the first release whose macOS tarballs hold `Adjutant.app`, the formula installs the app into its
prefix. Change `install`, add to `caveats` and `test`:

```ruby
  def install
    bin.install "adjutant"
    bin.install "adj"
    # Tarballs before the app shipped have no Adjutant.app.
    prefix.install "Adjutant.app" if OS.mac? && File.exist?("Adjutant.app")
  end
```

Build `caveats` from the existing text and a macOS-only part. An `if` written inside the heredoc would be
printed as text, so add the second heredoc in Ruby:

```ruby
  def caveats
    s = <<~EOS
      ...the existing text...
    EOS
    if OS.mac?
      s += <<~EOS

        The macOS app is in #{opt_prefix}/Adjutant.app. Copy it to /Applications so Spotlight and
        Launchpad find it:
          rm -rf /Applications/Adjutant.app.new
          ditto "#{opt_prefix}/Adjutant.app" /Applications/Adjutant.app.new &&
            rm -rf /Applications/Adjutant.app &&
            mv /Applications/Adjutant.app.new /Applications/Adjutant.app
        After brew upgrade, quit the app with Cmd+Q, copy it again, restart the server and open the app.
      EOS
    end
    s
  end
```

In `test`:

```ruby
    if OS.mac?
      system "codesign", "--verify", "--deep", "--strict", "#{prefix}/Adjutant.app"
    end
```

Why it looks like this:

- The app is in the formula and not in a cask, because a formula download is not quarantined: the ad-hoc
  signature (`bundle.macOS.signingIdentity` `"-"` in `app/src-tauri/tauri.conf.json`) is enough, with no
  Developer ID or notarization. A cask would be quarantined.
- The test runs `codesign --verify` on the installed copy, because Homebrew may rewrite Mach-O files in
  the keg after install, which would break the bundle's seal. On the first release with the app, run
  `brew install` and `brew test` (step 5) before announcing it.
- The caveats copy the app instead of linking it, because Spotlight does not index a symbolic link to
  an app.
