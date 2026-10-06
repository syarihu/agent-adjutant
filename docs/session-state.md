# Session state

A design for knowing what each agent session is doing (running, waiting on a permission prompt,
done with its turn, running sub-agents) without agent-proctor. It is for the people and agents who
will build it. Today that state comes only from [agent-proctor](https://github.com/syarihu/agent-proctor)'s
hooks and ledger, which means installing proctor and wiring its hooks into each agent's global
settings. This page covers the foundation: the hook receiver, the ledger, and how the hooks reach the
sessions adjutant starts. Showing the state on the board is a later piece of work. The sections
follow the questions in [#496](https://github.com/syarihu/agent-adjutant/issues/496); the last one
splits the work into issues.

## What comes over from proctor and what stays

proctor is a Swift package. Its CLI exists to feed its iTerm2 sidebar app, and the hook receiver
and the ledger are the parts adjutant needs.

| proctor | Here |
|---|---|
| Hook receiver (`_touch`, `_subagent`) and its state machine | Comes over, as `adj hook` |
| Sub-agent tracking keyed by `agent_id`, with the parent's `done` held until the last one ends | Comes over |
| Notification types told apart (permission prompt against idle prompt) | Comes over |
| Status line relay (`_stats`: context use, rate limits, model) | Comes over, as a relay the user wires in (see [What comes from the status line](#what-comes-from-the-status-line)) |
| Pruning of dead sessions by pid and start time | Comes over, reusing `registry::liveness` |
| Ledger as one `state.json` for every repository under one lock | Not as it is: one file per session (see [The ledger](#the-ledger)) |
| Naming hint on `UserPromptSubmit` and `proctor title` | Stays. adjutant names its sessions when it starts them; a second hint would be injected twice next to proctor's |
| `proctor setup`, which prints a guide for the agent to merge by hand | Replaced by injection at launch and an `adj setup` that writes the hooks itself |
| `proctor worktree ls`, worktree conventions, `proctor-worktree` skill | Stays in proctor (see [Worktree conventions and the hub procedure](#worktree-conventions-and-the-hub-procedure)) |
| iTerm2 sidebar app, its reaper, read marks, approval watcher for Antigravity, notifications | Stays. The board is adjutant's view, and its sessions tab is a later issue |
| `attach`, `rm`, avatars, logs | Stays |

Two behaviours of proctor's receiver carry over as rules, because each one fixes something seen in
practice:

- A row is created only by a `running`, `waiting` or `idle` event. A stray `done` or `clear` (from a
  session that ended before its row was written) never registers one.
- `SessionStart` on an existing row changes nothing but the pending request. It also fires on
  resume, compaction and `/clear`, and must not wipe a turn in progress.

## The ledger

**Where.** One JSON file per agent session under the state dir:
`agent-sessions/<session id>.json`, with `<session id>.lock` beside it. proctor's single file under
one `flock` serialises every agent on the machine; `PostToolUse` fires after every tool call, so
the per-session lock keeps one busy worker from slowing the others. The owner is `registry`
(rank 3), whose charter is "who is running and where": the receiver writes only this record, so the
operation sits in the lowest module that holds it. The store is part of `registry`'s private
`store.rs`; no other module builds these paths.

**Key.** The agent's own session id (`session_id` in a Claude Code payload). adjutant already mints
one for every session it starts (`registry::new_session_id`, passed as `--session-id`) and saves it
in `sessions/<slug>.json` for a hub and `<worktree>/.claude/adjutant-session.json` for a worker. So
a ledger row joins to a hub or worker by that id, never by worktree or cwd: two sessions can share a
worktree, and a hub shares the main checkout with anything else run there. A session started by a
custom runner whose template lacks `{sessionId}` still gets a row, keyed by the id the agent chose,
but it joins to no hub or worker. It shows as an unattached session, the same as one started by
hand.

The agent's id can change under a running process: `/clear` ends the session (`SessionEnd`) and
starts a new one with a new `session_id`, and `--fork-session` does the same. So adjutant also puts
the id it minted into the agent's environment at launch, as `ADJUTANT_SESSION_ID`. Hook processes
inherit the agent's environment, and that variable does not change on `/clear`, so the receiver
copies it into the row as `launchId`. A hub or worker is joined by `launchId` first and by
`sessionId` second, and the join survives a `/clear`.

**Shape.** Every field but the key is optional (rule 10 in [architecture.md](architecture.md)), and
keys this binary does not know are kept in `other`, as `WorkerRecord.other` does.

| Key | What |
|---|---|
| `sessionId` | The key, as above |
| `launchId` | `ADJUTANT_SESSION_ID` from the hook's environment: the id adjutant minted at launch |
| `agent` | `claude` first; `codex` and `agy` later |
| `status` | `idle`, `running`, `waiting`, `done` or `failed` |
| `pendingStatus` | A `done` or `failed` held back while sub-agents run |
| `cwd`, `worktree` | The payload's `cwd` and its `git rev-parse --show-toplevel` (absent outside git; the row is still written) |
| `launchedBy` | `injected` or `global`: which set of hooks wrote the row (see [Getting hooks into a session](#getting-hooks-into-a-session)) |
| `pid`, `psStarted` | The agent's process (`CLAUDE_PID` in the hook's environment) and its start time, for liveness |
| `createdAt`, `updatedAt` | `updatedAt` moves only when `status` changes, so tool activity does not reset "how long in this state" |
| `lastEventAt` | Moves on every event that writes; the "seen alive" mark |
| `activity` | The tool in use (`Edit: src/lib.rs`) |
| `request` | What a permission prompt is asking for, while `waiting` |
| `subagents` | Running sub-agents: `[{ id, type, startedAt, lastSeenAt }]`, keyed by `agent_id` |
| `finishedSubagents` | `{ agent_id: time }` for five minutes after each stop, so a late event cannot bring one back |
| `model`, `contextPercent`, `rateLimits` | From the status line relay, when wired in |

Every other record stays where it is. The worker record keeps its phase, and the task its status;
the ledger says what the agent is doing right now, and neither of the others does. `adj phase` is
what the worker says about its work; the ledger is what the agent's hooks say about the process.
They answer different questions, and neither replaces the other.

**Writes.** Load, change and save under the row's lock, replacing the file through a rename
(`infra::fs`); skip the write when nothing changed, as proctor does, so the file's mtime means
something. `git rev-parse` runs before the lock is taken.

**Pruning.** `SessionEnd` removes the row. For a row that ends without one (the tab was closed, the
process was killed), the receiver sweeps the other rows on `SessionStart` and `Stop`. Not on every
event: a sweep checks processes, and `PostToolUse` fires after every tool call.

- A row joined to a hub or worker record is dead when that record's liveness
  (`registry::hub_liveness` / `worker_liveness`, pid plus `ps` start time) says so.
- Any other row with a `pid` is dead when the pid is gone or its start time differs.
- A row with neither is dropped when it has not been `running` and `lastEventAt` is older than
  24 hours.

Claude Code sets `CLAUDE_PID` for hook commands, so every Claude Code row has a pid; the 24-hour
rule is for agents that give none (Codex, as proctor found).

**Reading.** `registry::agent_session(root, session_id)` and `registry::agent_sessions(root)`, taking
a state root like the other reads (rule 4), and `adj sessions [--json]` on the CLI so the foundation
can be checked end to end before the board shows it.

## Receiving hooks

One hidden command, `adj hook <agent>`, reads the payload from stdin and takes the event from the
payload's `hook_event_name`, so every entry in the generated settings is the same command. It lives
in `transport/cli/hook.rs`, calls `registry::record_agent_event`, and prints only what the event's
contract needs (`{}` for `PermissionRequest`, nothing otherwise).

It runs after every tool call, so it must be fast and must not fail the agent:

- It reads the state dir from `ADJUTANT_STATE_DIR` or the XDG default, as every `adj` command does,
  and resolves no config and calls no GitHub. `ADJUTANT_STATE_DIR` already reaches the agent:
  `lifecycle::forwarded_env` sends it to the tab and `lifecycle::agent_command` keeps it.
- Any error (unreadable payload, a lock that cannot be taken, a full disk) is written to stderr and
  the command exits 0. A hook that fails would show in the agent's transcript on every tool call.
- A corrupt row is moved aside to `<session id>.json.broken` and a new one started, as proctor does
  with its ledger.

The events, taken from proctor's table:

| Claude Code event | Matcher | What it records |
|---|---|---|
| `SessionStart` | | `idle` for a new row; on an existing row only clears `request` |
| `UserPromptSubmit` | | `running`; clears `pendingStatus` |
| `PostToolUse` | `*` | `running`, and `activity` |
| `PostToolUseFailure` | `*` | `running` (`PostToolUse` fires only on success) |
| `PermissionRequest` | `*` | `waiting`, and `request` (immediate; the permission `Notification` comes about 6 seconds later) |
| `Notification` | | `waiting` for `permission_prompt`, `elicitation_dialog` and unknown types; back from `waiting` to `idle` for `idle_prompt`; nothing for the rest |
| `Stop` | | `done`, or held in `pendingStatus` while sub-agents run |
| `StopFailure` | | `failed`, held the same way (it fires instead of `Stop` on rate limits and overload) |
| `SessionEnd` | | removes the row |
| `SubagentStart` | | adds the sub-agent by `agent_id` |
| `SubagentStop` | | removes it; applies `pendingStatus` when it was the last |

An event that carries `agent_id` comes from a sub-agent: it updates that sub-agent's `lastSeenAt`
and never the parent's `status`. `SessionEnd`, `SubagentStart`, `SubagentStop` and
`UserPromptSubmit` are not run in the background, so the process is not killed before it writes.

One known gap stays as it is in proctor: cancelling a permission prompt fires no hook, so the row
stays `waiting` until the `idle_prompt` notification about a minute later.

```mermaid
sequenceDiagram
    participant Agent as claude
    participant Hook as adj hook claude
    participant Reg as registry
    participant Board as board view
    Agent->>Hook: PostToolUse payload on stdin
    Hook->>Hook: parse, git rev-parse cwd (no lock)
    Hook->>Reg: record_agent_event(root, event)
    Reg->>Reg: lock row, apply, save if changed
    opt SessionStart or Stop
        Reg->>Reg: sweep dead rows
    end
    Hook-->>Agent: exit 0
    Board->>Reg: agent_sessions(root) on the next poll
```

## Getting hooks into a session

Both routes are needed, for different sessions.

**Injected at launch, for the Claude Code sessions adjutant starts.** adjutant writes one settings
file, `agent-hooks/claude-settings.json` under the state dir, holding the table above with the
absolute path of the running `adj` (`std::env::current_exe`; `PATH` is not reliable inside a hook),
and passes it with `--settings <file>`. The runner templates gain a `{settings}` placeholder,
rendered in `kernel::runner::worker_line` and `hub_line`; when a template does not name it and the
runner's agent is `claude` (`runner::agent_from_runner`), it is added the way `{prompt}` is. The
four default runners get it. Nobody edits a global settings file, and removing adjutant leaves
nothing behind in the agent's settings. The file is rewritten when its contents would change (a new
`adj` path after an upgrade), never while a session reads it: write to a temporary file and rename.

**Written once by `adj setup claude`, for everything else.** Sessions adjutant did not start, and
agents with no per-session settings flag (Codex and Antigravity read hooks only from a global file),
are reached only through the agent's global settings. `adj setup claude` adds the same table to
`~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`), appending to existing hook arrays and never
removing any, with `adj hook claude --global`; `adj setup claude --remove` takes out exactly the
entries it added and nothing else. It is opt-in: adjutant works without it.

Claude Code merges hook entries across settings levels rather than letting one replace another, and
`--settings` is one of those levels (above the user's, project and local files). So the injected
hooks run next to whatever hooks the user has, and the user's are untouched.

**No double counting between the two.** When both are present, a session adjutant started runs both
sets. Claude Code runs an identical handler defined in two settings files only once, but the two
commands differ (`--global`), and whether an inline or temporary `--settings` file takes part in that
is not documented, so adjutant does not rely on it. Instead the receiver run with `--global` returns
at once when `ADJUTANT_SESSION_ID` is set. adjutant sets that variable at launch (through
`runner::with_env`, which adjutant controls, so it is certain to be there) and hook commands inherit
the agent's environment, so the global copy steps aside only in sessions where the injected copy
runs.

**Next to proctor.** proctor's hooks write proctor's ledger and adjutant's write adjutant's, so
neither counts the other's sub-agents twice. What can happen is that the two disagree, for instance
if proctor's hooks are wrapped in a script that filters an event adjutant sees. That is acceptable:
adjutant reads only its own ledger. Running both costs a second short process per hook.

What the generated file looks like, shortened:

```json
{
  "hooks": {
    "PostToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "/opt/homebrew/bin/adj hook claude" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "/opt/homebrew/bin/adj hook claude" }] }
    ]
  }
}
```

## What comes from the status line

A session has one status line. If adjutant injected one through `--settings`, it would replace the
user's, so it does not. Instead `adj hook claude --status-line` reads the same stdin a status line
command gets and records `model`, `contextPercent` and `rateLimits` into the row, without creating
one. It prints nothing, so a user who wants these fields adds one line to their own status line
script, piping its stdin to it. Without it the rows simply lack those fields.

adjutant already depends on the user's status line in one place: `task::pick_review_engine` reads
`rate-limit-cache.json`, which a status line script writes. Once the relay exists, `adj
review-engine` reads the five-hour and seven-day figures from the newest row of the caller's agent
instead, and keeps the cache file as the fallback for a user who has it and not the relay. That is
a separate issue, so the review engine does not change in the foundation.

## Which agents, in what order

**Claude Code first.** It has per-session settings, the richest events, and payloads that carry
`session_id` and `agent_id`.

**Codex and Antigravity after the board shows Claude Code sessions.** Neither has a per-session
settings flag, so they are reached only by `adj setup codex` / `adj setup agy` writing their global
hook files. Their differences, from proctor's guides, decide the details of those issues:

- Codex treats hook stdout as a decision, so its hook must print nothing; it asks the user to trust
  new hook commands; it has no `StopFailure`; its rows rely on the 24-hour rule unless a pid is found.
- Antigravity has no hook for waiting on approval (proctor polls its conversation store) and reports
  sub-agents only through `PreToolUse` on `invoke_subagent`.

adjutant runs Codex as `Agent::Generic` today, and a `Generic` session gets no injection and no row
until its own issue lands.

## How adjutant uses it

None of this is in the foundation; it is listed so the ledger carries what it will need.

- **The board.** A sessions tab in the sidebar, and the session cards, read `agent_sessions` on the
  existing 2-second poll of `/api/state`. No push channel is added.
- **Waking.** `mail::read_screen` guesses an agent's state from a tmux screen, and works for neither
  iTerm2 nor `Generic`. A row in `waiting` or `running` says not to type now; `idle` or `done` says it
  is safe. The screen check stays for typing the line itself, and as the fallback with no row.
- **Gates and cards.** A worker in `waiting` on a permission prompt is waiting on a person even with
  no gate open; the card can say so. A worker whose row went `done` and stayed there with no phase
  change is a better "stuck" signal than the phase age alone.

## Worktree conventions and the hub procedure

The "Where proctor ends" section of `commands/adj-hub.md` is about worktree conventions
(`worktreeBase`, `branchPattern`, `copyFiles`), not session state, and it does not change. `adj
worktree-path` keeps answering where proctor is not installed; nothing here moves worktree
conventions into adjutant.

What changes later is the text that leans on proctor for progress: the hub procedure says a worker
"gets its own tab and its own proctor row" and that the proctor row shows progress, and the README's
"Optional neighbours" paragraph names proctor's session ledger. Once `adj sessions` exists, those say
to read adjutant's own sessions instead. That is a procedure change of its own, after the board shows
the state.

## What proctor does afterwards

proctor is a separate repository and keeps working as it is: its sidebar, its hooks and its ledger
do not depend on adjutant, and adjutant does not read them. Someone who wants the sidebar keeps
proctor's hooks, and the two ledgers live side by side. Whether proctor later reads adjutant's ledger
instead of keeping its own is proctor's decision, outside this repository; the row shape above is
plain JSON with one file per session, so it could.

## Where the new state lives

| Path (under the state dir) | What | Owner |
|---|---|---|
| `agent-sessions/<session id>.json` (+ `.lock`, `.json.broken`) | one agent session's state | registry (`store.rs`) |
| `agent-hooks/claude-settings.json` | the hook settings passed with `--settings` | lifecycle (written at launch) |

Both follow rule 10: new keys optional, unknown keys kept, names fixed once released.

## Implementation split

In order. Each one is a sub-issue of #496, and each lands on its own.

**1. adjutant has nowhere to record what an agent session is doing**

```markdown
## What happens

adjutant knows which hubs and workers it started, but nothing records what their agent is doing:
running, waiting on a permission prompt, done with its turn, or running sub-agents. The only source
is agent-proctor's ledger, which adjutant does not read.

## Proposal

Add the agent session ledger from `docs/session-state.md`: one file per session at
`agent-sessions/<session id>.json` in `registry`'s private store, the state machine for the Claude
Code events (sub-agents keyed by `agent_id`, a held `done`, notification types), the sweep of dead
rows, the hidden `adj hook claude` that reads a payload from stdin, and `adj sessions [--json]` to
read the rows. Tests feed recorded payloads through `adj hook` and check the rows.

Also confirm with a real session two facts the design takes from Claude Code's documentation
without it saying them outright: that the `session_id` in hook payloads is the one passed with
`--session-id`, and that hooks passed with `--settings` run alongside the user's own. Record the
answers in the doc.
```

**2. Hooks do not reach the Claude Code sessions adjutant starts**

```markdown
## What happens

Even with `adj hook` in place, a hub or worker adjutant starts only reports its state if the user has
added the hooks to their own Claude Code settings by hand.

## Proposal

Write `agent-hooks/claude-settings.json` under the state dir with the absolute path of `adj`, add a
`{settings}` placeholder to the runner templates (appended for a `claude` runner that does not name
it, like `{prompt}`), pass it in the four default runners, and set `ADJUTANT_SESSION_ID` to the
minted session id in the agent's environment, so a row keeps its hub or worker across `/clear`. Rewrite the file only when its contents change, through a rename.
```

**3. Claude Code sessions adjutant did not start cannot report their state**

```markdown
## What happens

A Claude Code session started by hand, or by a runner adjutant does not control, never calls
`adj hook`, so it has no row.

## Proposal

Add `adj setup claude`, which appends the hook table with `adj hook claude --global` to the user's
Claude Code settings without removing any existing hook, and `adj setup claude --remove`, which
takes out only those entries. `adj hook --global` returns at once when `ADJUTANT_SESSION_ID` is
set, so a session adjutant started is not recorded twice.
```

**4. Context use and rate limits never reach the session ledger**

```markdown
## What happens

The ledger knows each session's state but not its model, how full its context is, or how much of the
rate limits are used. Claude Code gives these only to the status line.

## Proposal

Add `adj hook claude --status-line`, which reads the status line's stdin, records `model`,
`contextPercent` and `rateLimits` on an existing row, and prints nothing. Document the one line to add
to a status line script. adjutant does not install a status line, since that would replace the user's.
```

**5. The board cannot show what each session's agent is doing**

```markdown
## What happens

The board shows a session's phase, last line and gates, but not whether its agent is running,
waiting on a permission prompt or done, and on iTerm2 it cannot tell at all.

## Proposal

Join `agent_sessions` to the board's sessions by session id in `board::view`, add the state to
`/api/state`, and show it in a sessions tab in the sidebar and on the session cards.
```

**6. adjutant decides when to type into a session from its screen alone**

```markdown
## What happens

Before waking a session, `mail::read_screen` guesses from a tmux screen whether the agent is busy. It
cannot read iTerm2 or a generic runner, and a permission prompt waiting with no gate open looks like
work in progress on the board.

## Proposal

Use the session's row when there is one: hold the wake while it is `running` or `waiting`, and show a
worker waiting on a permission prompt as waiting on a person. Keep the screen check as the fallback.
Also let `adj review-engine` take the rate limits from the newest row before `rate-limit-cache.json`.
```

**7. The hub procedure still points at proctor's row for a worker's progress**

```markdown
## What happens

`commands/adj-hub.md` says a worker's progress shows in its proctor row, and the README lists
proctor's session ledger as an optional neighbour, though adjutant now has its own.

## Proposal

Point those passages at `adj sessions` and the board. Leave "Where proctor ends" as it is: it is about
worktree conventions, which stay with proctor.
```

**8. Codex and Antigravity sessions cannot report their state**

```markdown
## What happens

Only Claude Code sessions have hooks into the ledger. Codex and Antigravity have no per-session
settings flag, and their hooks behave differently (Codex reads hook stdout as a decision; Antigravity
has no hook for waiting on approval).

## Proposal

Add `adj setup codex` and `adj setup agy` writing their global hook files, and teach `adj hook` their
payloads, with the differences listed in `docs/session-state.md`. One issue per agent if they turn out
to differ more than expected.
```
