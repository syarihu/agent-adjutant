English | [日本語](configuration.ja.md)

# Configuration & Runtime Reference

This document covers the advanced configuration options, command templates, environment variables, terminal backends, and session lifecycle mechanisms in agent-adjutant.

For a full commented JSON schema, see [`config.example.json`](../config.example.json).

---

## File Location & Precedence

Config file path: `~/.config/adjutant/config.json` (honours `$XDG_CONFIG_HOME` and `$ADJUTANT_CONFIG`).

Resolution precedence (most specific wins):
1. Repository-specific entry (`repos["owner/name"]`)
2. Default repository settings (`defaults`)
3. Top-level keys in `config.json`
4. Built-in defaults

The resolver never fails on unknown keys or invalid entries; it issues warnings accessible via `adjutant config` and falls back safely.

---

## Command Templates & Placeholders

All command options in `config.json` are command templates. Placeholders are **already shell-quoted** when substituted: do not add quotes around them.

```jsonc
// Example: DO NOT add quotes around {command} or {title}
"terminal": {
  "spawn": "my-terminal --title {title} -- {command}"
}
```

### Staging Long Command Lines
When an expanded command line is longer than 900 bytes (common when passing large `PATH` variables via `agentEnv`), adjutant writes it to an `adjutant-spawn-*.sh` script in the system temporary directory (`TMPDIR`, else the platform default) and runs `sh <that script>` instead. A long line typed into the terminal can arrive with a chunk dropped from the middle, so the short line avoids running a mangled command. Dry runs are never staged.

---

## Environment Variables

- `ADJUTANT_CONFIG`: Path to an alternate configuration file. Forwarded automatically to tabs opened by `adj hub --tab` and `adj work`.
- `ADJUTANT_STATE_DIR`: Custom state directory (default: `~/.local/state/adjutant`). Relative paths are resolved against the repository's main checkout and passed as absolute paths.
- `ADJUTANT_HUB`: Identifies the hub session. When launching workers, `adj work` records the hub's ID into the worktree so that reports reach the correct hub.

---

## Terminal Backends

### 1. Default: built-in tmux backend
With no `terminal.preset` (or `"terminal": { "preset": "tmux" }`), adjutant is a first-class tmux controller. It requires tmux: `adj work`, `adj hub --tab` and a hub start from the board stop with a one-line error naming tmux when it is not installed. An unknown `preset` value is a warning in `adj config` and falls back to tmux. A `terminal.spawn` template still overrides the preset.
- **Detached Window Spawning**: Workers spawn in background windows (`tmux new-window -d`) without stealing user focus. The process hierarchy is monitored for 3 seconds; if the agent exits immediately, the slot is freed and the error is reported.
- **Process-to-Pane Mapping**: adjutant inspects PIDs, TTYs, and parent-child process relationships to locate tmux panes without wrapper scripts.
- **Waking with Screen Inspection**: Before typing into a pane, adjutant inspects the screen (`tmux capture-pane`). It only sends keys if the agent prompt is completely empty and idle. If an interactive prompt, menu, question, or in-progress typing is detected, input is withheld and a desktop notification is triggered instead.
- **Focus & Close**: `adj focus` selects the window and pane (`select-window` / `select-pane`). `adj close` terminates the worker window (`kill-window`).
- **Attaching**: Connect anytime using `tmux attach -t adjutant` or iTerm2 control mode via `tmux -CC attach -t adjutant`.
- **CLI Subcommands**: Direct management via `adj tmux` (`pane`, `spawn`, `wake`, `focus`, `close`).
- **Environment variables**: `$ADJUTANT_TMUX_SESSION` (overrides default `"adjutant"`) and `$ADJUTANT_TMUX_SOCKET` (passes `tmux -L <socket>`).

A plain `adj hub` typed outside tmux still runs the hub in that terminal, but the board cannot open its terminal, so use `adj hub --tab` to start one in the tmux session.

### 2. iTerm2 (`terminal.preset: "iterm2"`, macOS)
Uses AppleScript to create tabs, adjust titles, and send text inputs. `{pid}` and `{tty}` represent operating system identifiers (`ttys004`), not terminal pane indices. On other platforms the preset still applies, but `adj config` warns that iTerm2 runs only on macOS.

### Migrating from the iTerm2 default
iTerm2 used to be the backend when `terminal.preset` was not set. After upgrading, such a setup gets tmux windows for new workers. To keep the old behaviour, set:

```json
{ "terminal": { "preset": "iterm2" } }
```

A config that sets only `terminal.spawn` keeps its own spawn, but focus, close and wake now go to the tmux backend too; add `"preset": "iterm2"` if they should stay on iTerm2. Sessions that are already running keep the backend they were recorded with, so close, focus and wake still reach them. The resident server reads the config again on the next board request after the file changes, but it keeps running the binary it started with, so run `adj server restart` after upgrading.

---

## Runner Templates & Session Resuming

### Runners
- `agentRunner`: Template to start a task worker.
  - Default: `claude --session-id {sessionId} --permission-mode auto {settings} {prompt}`
- `hubRunner`: Template to start a repository hub.
  - Default: `claude -n {name} --session-id {sessionId} --permission-mode auto {settings} {prompt}`
- `reviewerRunner`: Template to start a sandboxed PR reader (AI 下読み).
  - Drops setting sources and MCP configs (`--setting-sources '' --strict-mcp-config --permission-mode dontAsk`), locking the session into read-only inspection.
- `agentResumeRunner` / `hubResumeRunner`: Invoked when resuming existing conversations via `--resume`.

### Agent Model Selection (`--model`)
- **CLI flag**: `--model <MODEL>` on `adj work` or `adj worker` selects the agent model for that worker session.
- **`{model}` placeholder**: Worker runner templates (`agentRunner`, `agentResumeRunner`) can place `{model}` where the model argument belongs. The value is shell-quoted when rendered; do not quote it in the template. If no model is specified, `{model}` is removed. When the preceding word is a model option (exactly `-m`, or starting with `--` and containing `model`, e.g. `--model {model}`, `-m {model}`, `--agent-model {model}`) or the word is `--*model*={model}`, the option word is removed together with the placeholder so no empty argument or double space remains.
- **Claude runners without `{model}`**: When a runner invokes `claude` and does not pass `--model` of its own, `--model <MODEL>` is inserted right after `claude`. A runner that cannot take a model (a non-claude runner without `{model}`, or a claude runner that already passes `--model`) is refused before starting when `--model` was given.
- **Allowed characters**: 1 to 128 ASCII characters from `A-Z a-z 0-9 . _ - : / @ [ ]`, starting with a letter or digit (e.g. `opus`, `sonnet`, `claude-opus-5-5`, `sonnet[1m]`, `openai/gpt-5`).
- **Persistence across resume**: The chosen model is saved in `.claude/adjutant-session.json`. Resuming with `adj work --resume` reuses the saved model by default, while an explicit `--model` on `--resume` replaces it. When `agentResumeRunner` cannot take the saved model, the session is resumed without it and a warning says so.

### Session Lifecycle and Auto-Resume
- **Auto-resume window (`hubAutoResumeHours`)**: A plain `adj hub` automatically reopens the previous session if it ended within this window (default: 3 hours). After this window expires, a clean session is created.
- **Idle readers (`readerIdleMinutes`)**: A PR reader (AI 下読み) that has handed in its read-through and that nobody has talked to for this many minutes (default: 60) is closed by the resident board server; the read-through, the drafts and the record stay, only the session goes. Nobody talking is told by the `updatedAt` of the reader's agent-session row, which moves whenever the session's status changes (a turn starts or ends, a permission prompt opens or is answered). Only a session whose last turn has ended (`idle`, `done` or `failed`) is closed, never one that is running or waiting for permission, nor one with no status or one this version does not know; a reader without an agent-session row (a `reviewerRunner` without adjutant's hooks) is not closed this way either. Like the other closes, it closes the reader's tmux pane, so a reader outside tmux stays open and the board server logs why. `0` turns it off.
- **Session IDs**: Runners that accept `{sessionId}` persist the identifier in `~/.local/state/adjutant/sessions/` (for hubs) or `.claude/adjutant-session.json` (for workers).
- **Liveness tracking**: Active hub sessions update a heartbeat file (`sessions/<slug>.alive`) once every minute and upon pipe termination, preventing stale sessions from being misidentified as alive.

---

## Desktop Notifications

`notification` template placeholders: `{title}`, `{message}`, `{nwo}` (the repository name `owner/name`).

- **Unset (the default)**: one channel rings, chosen in this order:
  1. The desktop app, if it polled within 90 seconds and can show notifications: only the app rings, as a native notification.
  2. Otherwise a board page in a browser that polled within 90 seconds and has notification permission: only that page rings.
  3. Otherwise `terminal-notifier` (click-to-activate terminal behavior), if it is installed. If it is not, nothing rings: there is no `osascript` fallback, because macOS credits that to Script Editor.

  Whether a client is "open" depends on the browser's notification permission alone, not on the 確認待ち setting. A browser page rings nothing while the app is open.

  Waits and gates follow the same order but also need that client's 確認待ち on. If the target client polls the same server process that watches the board, it rings them itself; if it is open on another server process (it cannot see this board), the server queues a notice for it, once. With the target's 確認待ち off, a wait or gate rings nowhere. "Open" is read from every board server on the machine, not only the one the page talks to. An app window that macOS throttles for more than 90 seconds (hidden, say) counts as closed, so a browser page that is open may ring meanwhile.
  
  With neither open, a wait or a gate that needs you is still on the board's TODO when you next look, but a message from `adjutant send` / `adjutant tell` or `adj notify` is not kept: it is shown only if `terminal-notifier` is installed. A message queued for the app or a page that is not taken within about 100 seconds (the window closed, say) is rung through `terminal-notifier` if installed, and dropped otherwise.
- **A configured command**: runs as it always did, and `false` turns notifications off. Neither looks at the app or the page, except that a wait or a gate is left to a board page of the same server while one is open.
- **Linux**: Set `"notification": "notify-send {title} {message}"`. With a board page open, an unset `notification` queues for the page.
- **Trigger Conditions**:
  - Direct messages via `adjutant send` / `adjutant tell`
  - Sessions waiting on permission prompts or user questions for more than a few seconds
  - Newly opened gates
  - Note: With `notification` unset, the app or board page that is open rings them (see above). With a command configured, they are suppressed while a web board tab of that server is open and polling.

---

## Who Merges a Worker's PR (`merge`)

`merge` is set per repository (or under `defaults`):

- `"person"` (default): the person merges the PR a worker opens. A worker that has finished asks its hub to clean up the worktree, and that request waits for the person's answer.
- `"hub"`: the hub merges it. When the worker says the PR is ready (no open `must` finding, `verify` passed, review bots handled), it sends `done` without the cleanup gate and the hub merges once the PR is open and not a draft, its checks have passed, it is `CLEAN` against its base, and nothing has been added to it since the `done` was sent. It merges with a merge commit (`gh pr merge --merge`), then removes the worktree. Anything it cannot settle itself (a review still owed, a merge GitHub refuses, checks still pending after 30 minutes) is left for the person; what the worker can fix (red CI, a late review) is sent back to it.

The hub reads `merge` from its own config, not from what a worker writes. Any other value is reported in `warnings` and treated as `"person"`.

---

## Running Tasks Without a Person (`autopilot`)

`autopilot` is set per repository (or under `defaults`) and is `false` unless you turn it on. With `true`, the hub takes a task from the queue to a merged PR with nobody at the board.

What it does:

- **Presets `merge` to `"hub"`.** The first of these that says something wins: the repository's own `merge`; else the repository's `autopilot` (`false` falls back to `defaults`' `merge`, else `"person"`); else `defaults`' `merge`; else `defaults`' `autopilot`; else `"person"`. So a repository can turn a default `autopilot` off, or keep `merge: "person"` while the hub does the rest. `adj config` shows the settled `merge` and `autopilot`.
- **Defaults Stop at to `none`** where the hub writes it: a board request it reads, a report it files, a request in its tab with nothing said. A Stop at already on a record, or one the person named, is kept; a task handed to Jules keeps `plan`.
- **Files and queues reports.** A worker's report with no `## Scope` is filed and queued instead of only filed. A report that is clearly the same bug as an open issue becomes a comment on it. The queue starts it in order (this needs `maxWorkers`).
- **Skips the start check** for a board request whose reading has no open point.
- **Holds a task that overlaps one in progress.** Before starting a task the hub compares it with the tasks in progress (the issue bodies, and the other worker's plan steps). On an overlap it writes the note `Waiting for task <key>: overlaps <area>`, `adj task next` passes over it, and the hub clears the note when the other task is done. Stacking on the other's branch is not used.

What still stops for a person: failed `verify`; every merge the hub cannot settle itself; a missing or interactive `issueCreate.command`; a report for another repository; a `needs-user` or a question the report cannot settle; a worker's proposal to split its task; a Jules plan; the start check of a task with `autoStart: false`; a board reading with an open point; editing an existing Jira ticket.

`autopilot` with no `maxWorkers` is reported in `warnings`: the hub only starts queued tasks when a slot is free, so it will not start them on its own. `autopilot: true` beside `merge: "person"` in one place is reported too (the `merge` wins). A value that is not `true` or `false` is reported and read as `false`. A repository that is not registered is never on autopilot.

Not there yet: the board neither shows nor switches `autopilot`, and the config is the only place to set it; there is no pause; the hub does not pick new work from the tracker; a parent task's hub reads the repository's setting and has none of its own.
