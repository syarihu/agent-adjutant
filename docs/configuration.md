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
When an expanded command line exceeds approximately 900 characters (common when passing large `PATH` variables via `agentEnv`), adjutant stages the script in `/tmp/adjutant-spawn-*.sh` and executes `sh /tmp/adjutant-spawn-*.sh`. This avoids shell truncation issues. Dry runs are never staged.

---

## Environment Variables

- `ADJUTANT_CONFIG`: Path to an alternate configuration file. Forwarded automatically to tabs opened by `adj hub --tab` and `adj work`.
- `ADJUTANT_STATE_DIR`: Custom state directory (default: `~/.local/state/adjutant`). Relative paths are resolved against the repository's main checkout and passed as absolute paths.
- `ADJUTANT_HUB`: Identifies the hub session. When launching workers, `adj work` records the hub's ID into the worktree so that reports reach the correct hub.

---

## Terminal Backends

### 1. Default (iTerm2 on macOS)
Uses AppleScript to create tabs, adjust titles, and send text inputs. `{pid}` and `{tty}` represent operating system identifiers (`ttys004`), not terminal pane indices.

### 2. Built-in tmux Backend (`terminal.preset: "tmux"`)
Configuring `"terminal": { "preset": "tmux" }` turns adjutant into a first-class tmux controller:
- **Detached Window Spawning**: Workers spawn in background windows (`tmux new-window -d`) without stealing user focus. The process hierarchy is monitored for 3 seconds; if the agent exits immediately, the slot is freed and the error is reported.
- **Process-to-Pane Mapping**: adjutant inspects PIDs, TTYs, and parent-child process relationships to locate tmux panes without wrapper scripts.
- **Waking with Screen Inspection**: Before typing into a pane, adjutant inspects the screen (`tmux capture-pane`). It only sends keys if the agent prompt is completely empty and idle. If an interactive prompt, menu, question, or in-progress typing is detected, input is withheld and a desktop notification is triggered instead.
- **Focus & Close**: `adj focus` selects the window and pane (`select-window` / `select-pane`). `adj close` terminates the worker window (`kill-window`).
- **Attaching**: Connect anytime using `tmux attach -t adjutant` or iTerm2 control mode via `tmux -CC attach -t adjutant`.
- **CLI Subcommands**: Direct management via `adj tmux` (`pane`, `spawn`, `wake`, `focus`, `close`).
- **Environment variables**: `$ADJUTANT_TMUX_SESSION` (overrides default `"adjutant"`) and `$ADJUTANT_TMUX_SOCKET` (passes `tmux -L <socket>`).

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

### Session Lifecycle and Auto-Resume
- **Auto-resume window (`hubAutoResumeHours`)**: A plain `adj hub` automatically reopens the previous session if it ended within this window (default: 3 hours). After this window expires, a clean session is created.
- **Session IDs**: Runners that accept `{sessionId}` persist the identifier in `~/.local/state/adjutant/sessions/` (for hubs) or `.claude/adjutant-session.json` (for workers).
- **Liveness tracking**: Active hub sessions update a heartbeat file (`sessions/<slug>.alive`) once every minute and upon pipe termination, preventing stale sessions from being misidentified as alive.

---

## Desktop Notifications

`notification` template placeholders: `{title}`, `{message}`, `{nwo}` (the repository name `owner/name`).

- **macOS Priority**: Prefers `terminal-notifier` (allowing click-to-activate terminal behavior) and falls back to `osascript`.
- **Linux**: Set `"notification": "notify-send {title} {message}"`.
- **Trigger Conditions**:
  - Direct messages via `adjutant send` / `adjutant tell`
  - Sessions waiting on permission prompts or user questions for more than a few seconds
  - Newly opened gates
  - Note: Notifications are suppressed if a web board tab is open and actively polling.
