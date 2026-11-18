# Board demo recordings

The clips of the web board in `docs/images/board-*.gif` / `.mp4` are recorded from the real board,
built from this checkout, in a fictional world: a repository `acme/shop-app`, and scripted agents in
place of Claude Code. Re-record them when the board changes.

## Requirements

- `cargo build` (the clips run `target/debug/adj`)
- tmux, ffmpeg, Node.js and Google Chrome
- `npm install` in `scripts/demo/record` (it installs `playwright-core` only; the browser is your Chrome)

## Recording

```bash
scripts/demo/record.sh request-plan ja   # one clip, one language
scripts/demo/record-all.sh               # every clip, ja and en
```

Each run builds a fresh world under `/tmp/adj-demo`, starts the server and the hub there, plays the
clip in a headless Chrome, and writes `docs/images/board-<clip>.mp4` and `.gif` (`.ja` before the
extension for Japanese). The board itself is in Japanese in both languages; the captions follow the
language.

| Clip | What it shows |
| --- | --- |
| `request-plan` | 「依頼する」, the hub's start check, and approving the worker's plan |
| `investigate-result` | An investigation that comes back as a report, with an HTML report attached |
| `review-own` | An AI read-through of the PR a worker opened, handed back to the worker |
| `review-others` | An AI read-through of a PR you were asked to review, and a pending review |
| `boards-list` | 「工程」 in its list layout, a task on every stage, and switching to the columns |
| `search-palette` | Finding a session with ⌘K and opening it |

## Screenshots of the macOS app

`docs/images/app-window.png` is the macOS app on the same world. `app.sh` builds a fresh world,
seeds the board as `boards-list` and `review-others` do, and opens the app on it:

```bash
make app-deps   # the Tauri CLI 2, when cargo tauri is missing
(cd app/src-tauri && cargo tauri build --bundles app --config '{"identifier":"dev.syarihu.adjutant.demo"}')
scripts/demo/app.sh
```

With `CARGO_TARGET_DIR` set, the bundle is under it: pass its path to `app.sh`
(`scripts/demo/app.sh "$CARGO_TARGET_DIR/release/bundle/macos/Adjutant.app"`).

The app is built with an identifier of its own: with the real one it would hand itself over to an
Adjutant that is already running, whose single-instance socket is named after the identifier.
`app.sh` refuses an app with the real identifier. It runs the app under the demo's `HOME`, so the
settings it reads (the demo's `adj`, the material background, no global shortcut) are the demo's.
`env.sh` unsets `ZDOTDIR`, so the login shell the app runs to find `adj` reads no startup file of
yours that could point it back at the real state.

To show a gate, ask the repository hub in 「依頼する」 for
`クーポンが有効期限の当日に使えないので直してほしい`: the scripted hub opens its start check, then the
worker its plan. Take the screenshot of the whole screen, not of the window, so the desktop shows
through the window's background, and crop the menu bar. Quit the app, then run `stop.sh`.

## How it works

- `env.sh` points everything at `/tmp/adj-demo`: `ADJUTANT_STATE_DIR`, `ADJUTANT_CONFIG`, `HOME`,
  `GH_CONFIG_DIR`, the git config and a tmux socket of its own. A recording never reads or writes
  your own state, config, GitHub login or tmux server.
  `env.sh` and the scenes refuse to go on under a shell other than bash, and the scenes also
  outside a world `setup.sh` built, so a scene sourced by hand cannot reach the real state.
- `gh-stub.py` stands in for `gh`. It answers from the pull requests in `prs/` and fails every other
  call, so nothing reaches GitHub.
- `agents/claude` is the hub, the worker and the reader (`hubRunner`, `agentRunner` and
  `reviewerRunner` in `demo-config.json`). It draws a screen like an agent's, reports through
  `adj hook claude` as a Claude Code session does, and does its work with the real `adj` commands
  (`adj pending`, `adj gate open`, `adj work`, `adj review submit`). `agents/scenes.sh` is the script
  each of them plays.
- `scenes/<clip>.sh` puts the board in the state a clip starts from (a worker already running, the
  review requests synced).
- `record/clips/<clip>.mjs` is what is clicked and when, and the captions. `record/lib.mjs` draws the
  pointer, which a headless browser does not show, and takes frames through the DevTools screencast.
  The captions are drawn as a band under the video (`record/captions.mjs`), so they never cover the
  board's dialogs.
