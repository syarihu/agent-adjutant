English | [日本語](README.ja.md)

# agent-adjutant

<img alt="agent-adjutant-logo" src="docs/images/agent-adjutant-logo.png" />

A task hub for coding agents, as one binary.

![agent-adjutant demo](docs/images/demo.gif)

`adjutant` is a repository's 副官 — its adjutant: it hands work out to workers and takes
their reports back in. The hub picks a task, cuts a worktree, writes a brief and starts a
worker in a new tab; a worker that trips over an unrelated bug hands it back rather than
fixing it or filing it itself. Both halves of that are procedures, and the procedures ship
inside the binary.

## Why a binary that serves prompts

The procedures used to be markdown files copied into each agent's commands directory. A copy
drifts: upgrade the tool and the copies stay behind, one per config directory, all subtly
different. Served over MCP they are a pointer instead — one upgrade moves every caller.

The mechanical parts (deriving the hub's name, resolving the config, opening a tab, moving a
message) used to be prose repeated across three procedure files, which is a rule with three
versions. They are commands now, and the procedures call them.

## Install

Via Homebrew:

```bash
brew install syarihu/tap/agent-adjutant # both binaries: `adjutant` and the short `adj`
adjutant install-mcp                   # registers the MCP server with Claude Code (user scope)
adjutant install-mcp --target json          # or print the JSON for another client
```

Via Cargo:

```bash
cargo install --git https://github.com/syarihu/agent-adjutant # both binaries: `adjutant` and the short `adj`
# or from a local checkout:
#   cargo install --path .
# or without cargo install:
#   cargo build --release && cp target/release/adjutant target/release/adj ~/bin/
```

`install-mcp` performs the registration rather than printing instructions for someone to
follow: it runs `claude mcp add`, because the tool that owns a config file is the one that
should write it. The `json` target is the escape hatch for clients this does not know, and
is the only path that asks you to paste anything.

**Install before registering.** The registration records `adjutant` when PATH already
resolves to the binary you ran, and its absolute path otherwise — so running `install-mcp`
out of a build directory pins that build forever, and `cargo clean` then breaks the server.

**`adj` is the same program under a shorter name** — both binaries are installed, and every
example below works either way. `adj work` even starts its worker tabs as `adj worker`,
because a command that names itself uses the name it was called by.

## Two modes

Shell-side, for the places that run *before* an agent exists — launchers, hooks, the
procedures' own `Bash` steps (`adj` everywhere, if you prefer):

| | |
| --- | --- |
| `adjutant hub [--tab] [--resume\|--new] [--no-dashboard\|--dashboard]` | start this repo's hub, in the main checkout, once — resuming the last session if it ended within `hubAutoResumeHours` (`--tab`: open a tab and start it there, rather than becoming it in this one; `--resume`: reopen the last session however long ago it ended; `--new`: start a fresh one even so; `--no-dashboard`: skip the listing it collects at startup, `--dashboard`: collect it anyway — both override `startupDashboard`) |
| `adjutant hub-name [--json]` | the hub's session name — the address a report goes to |
| `adjutant config` | the resolved config for this repo, as JSON |
| `adjutant pending [--json\|--read N\|--ack N\|--path]` | what is waiting for the hub |
| `adjutant send --subject … --body …` | hand a message to the hub (body may come on stdin) |
| `adjutant work --worktree … --title … [--resume]` | open a tab and start a worker there (`--resume`: reopen the worker session saved in that worktree) |
| `adjutant worker --worktree … [--resume]` | become the worker (what `work` opens a tab to run; `--resume` inside a worktree reopens its saved session) |
| `adjutant tell --worktree … --subject …` | leave a message for that worktree's worker |
| `adjutant outbox [--clear]` | what the hub has left for the worker here |
| `adjutant spawn --cwd … -- cmd …` | open a tab and run something in it |
| `adjutant focus` | raise the running hub's tab; exit 1 if there is none |
| `adjutant close --worktree …` | close the tab that worktree's worker is sitting in; exit 1 if it is still there |
| `adjutant ide --worktree …` | open a worktree in the configured editor |
| `adjutant title --title …` | name the tab this process is in (the hub names its own) |
| `adjutant notify --message …` | tell the human something happened |
| `adjutant worktree-path --name …` | the branch, path and the main checkout to create it in |
| `adjutant serve [--port N] [--no-open]` | serve this repository's board at `http://127.0.0.1:4577` (`--port 0` picks a free one) |
| `adjutant task add\|list\|show\|update` | the records that board is a view of |
| `adjutant gate open\|list\|show\|answer` | what an agent has put up for a person, and the answer back |
| `adjutant hub-stop` | clear this repo's hub record |

Agent-side (`adjutant mcp`), the same machinery as seven tools and three prompts:

- **prompts** — `adj-hub` (run the hub), `adj-worker` (take a task from brief to handover),
  `adj-report` (hand a bug you found to the hub). Claude Code exposes these as
  `/mcp__adjutant__adj-hub` and so on.

- **tools** — `adjutant_config`, `adjutant_hub_status`, `adjutant_send`, `adjutant_pending`,
  `adjutant_tell`, `adjutant_outbox`, `adjutant_skill`. The last one serves the same
  procedure text as the prompts, because MCP prompt support is uneven across agents and a
  procedure nobody can fetch is a procedure nobody follows.

The names are short where a person types them and long where something reads them back:
`adj` on the command line and `adj-…` for the prompts, against `adjutant` for the MCP server
and `adjutant_…` for the tools. A registration or a resolved config that says `adjutant` is
one you can still identify a year later.

Every command that answers *about a repository* takes `--repo owner/name` (`outbox`, `mcp`
and `install-mcp` do not — they are not about one); without it the repository is read from
the origin remote of whichever checkout you are standing in, worktrees included.

A repository can have more than one hub. `--hub <id>` names which one: it moves the address
— the session name, the inbox, the record — and nothing else. The configuration is still
looked up under `owner/name`, so a second hub of a registered repository keeps its task
sources, issue keys and verify command. Without `--hub` you get the repository's own hub, at
exactly the address it has always had — except inside a worktree `adj work` opened, where a
command that *addresses* a hub reads the identifier out of that worktree's record. The ones
that *start* something — `hub`, `work`, `worker` — never do: a hub launched from inside a
worktree, and a worker registering in the worktree its tab was opened at, would both be
reading a record that belongs to somebody else.

Nothing has to repeat the identifier afterwards. `adj hub --hub <id>` puts `ADJUTANT_HUB` on
the command line it starts the agent with, so every `adj` call and every MCP tool call that
agent makes addresses the hub it is; and `adj work` writes the identifier into the worktree
it opens, so a worker reporting from there reaches the hub that dispatched it without being
told where to send.

`--no-dashboard` and `--dashboard` ride the same channel: they become
`ADJUTANT_STARTUP_DASHBOARD` on that command line, and `adjutant config` folds the value in
before it answers, so the procedure reading `settings.startupDashboard` sees the flag the hub
was started under rather than the file it disagrees with. With `--tab` there is no such
variable to see: a terminal is handed a command line and nothing else, so the flag is
forwarded to the `adjutant hub` that runs in the new tab, and that one builds the
environment. Same answer, one process later — which is why the two dry runs do not print the
same thing.

### Resuming after a restart

Updating the agent, or a crash, ends the hub and its workers. A hub that ended within the last
`hubAutoResumeHours` (3 by default) comes back on a plain `adj hub`; past that, `adj hub`
starts a new, empty conversation, so the first hub of the morning is a clean one. `--resume`
reopens the one it had whenever it ended, and `--new` starts fresh even inside the window:

```bash
adj hub --resume                  # this repo's hub, from anywhere in the repo
adj hub --resume --hub ALPHA-233  # a parent task's hub — the identifier is never guessed
adj worker --resume               # in a worktree: the worker that was working there
```

Every start makes up a session id and hands it to the agent (`--session-id {sessionId}` in
the default runners). The id is saved beside the records, not in them: the hub's under
`sessions/` in the state directory, the worker's in the worktree's
`.claude/adjutant-session.json`. That is why `hub-stop` and `close`, which clear the records,
leave it alone. `--resume` reopens that id with `hubResumeRunner` / `agentResumeRunner`
(Claude Code's `--resume` by default), goes through the same claim as a fresh start, and tells
the agent to check its inbox or outbox for whatever arrived while it was gone.

When the hub ended is written by the hub's own MCP server. `adj hub` puts
`ADJUTANT_HUB_SESSION` on the line it `exec`s, the agent's `adjutant mcp` inherits it, and
that server records the session as alive every minute and once more when the agent closes
its pipe — in `sessions/<slug>.alive`, a file of its own so that an old hub's last beat can
never overwrite a new hub's saved session. The same server runs under every session on the
machine, and only the one carrying the variable writes anything; `adj worker` strips it
before it starts an agent. With no MCP server under the hub nothing says when it ended, and
`adj hub` starts fresh rather than guessing. The same goes for a hub started by a `hubRunner`
of your own with no `hubResumeRunner` beside it: the built-in resume command would reopen it
without whatever your runner adds, so it is only resumed when `--resume` asks.

Workers are only resumed when asked: `adj work` is how a hub hands over a new brief, and
coming back to an old conversation there would bury it.

A resumed worker goes back under the hub that dispatched it, as saved — ahead of
`ADJUTANT_HUB`, which the tab it is typed in may have inherited from a different hub. A hub
asked to resume with nothing saved lists the hubs of the repository that do have a session.
A runner with no `{sessionId}` starts sessions nobody can resume; that is not an error until
`--resume` is asked for.

`instructions` is five lines. The 1500 lines of procedure matter only while a hub or a worker
is running, and both fetch them on purpose; the one thing worth always-on context is that a
worker is allowed to report a bug it did not come to fix.

## Permissions

A procedure served over MCP cannot carry a tool allowlist. As a slash-command file it could
— `allowed-tools:` in the frontmatter — and that is the one thing lost in moving the
procedures into the binary.

So **both sessions start unattended by default**, hub as well as worker. A worker must not
stop because it has a build to finish; a hub must not stop because a hub waiting for
approval is a hub not reading its inbox, and nobody is watching that tab — which is the
entire premise. This does not remove the questions that matter: the procedures' own
`AskUserQuestion` checkpoints (file this issue? start work on it?) are untouched. What goes
away is being asked whether `gh issue view` may run.

If you would rather the hub asked, take the mode back out of both hub runners —

```jsonc
"hubRunner": "claude -n {name} --session-id {sessionId} {prompt}",
"hubResumeRunner": "claude -n {name} --resume {sessionId} {prompt}"
```

— and pre-approve what the procedures reach for, in `~/.claude/settings.json`. Note that
this list is a snapshot: it drifts the moment a procedure reaches for something new, and the
symptom is the hub going quiet.

```jsonc
"permissions": { "allow": [
  "Bash(adj:*)", "Bash(adjutant:*)",
  "Bash(git:*)", "Bash(gh:*)",
  "Bash(cat:*)", "Bash(ls:*)", "Bash(mkdir:*)", "Bash(mv:*)", "Bash(cp:*)",
  "Bash(sed:*)", "Bash(awk:*)", "Bash(printf:*)", "Bash(date:*)", "Bash(ps:*)",
  "Bash(basename:*)", "Bash(open:*)", "Bash(which:*)",
  "Bash(proctor:*)", "Bash(lk:*)", "Bash(codex:*)",
  "mcp__adjutant__adjutant_config", "mcp__adjutant__adjutant_hub_status",
  "mcp__adjutant__adjutant_send", "mcp__adjutant__adjutant_pending",
  "mcp__adjutant__adjutant_tell", "mcp__adjutant__adjutant_outbox",
  "mcp__adjutant__adjutant_skill"
]}
```

The hub runs in the main checkout, so an unattended one can touch that working tree. The
procedure forbids it from implementing anything there — everything goes out to a worker in
its own worktree — but that is a rule in prose, not a sandbox.

## Config

`~/.config/adjutant/config.json` (`$XDG_CONFIG_HOME` and `ADJUTANT_CONFIG` are both honoured).
See **`config.example.json`** — its `//` keys are the schema documentation, and they are
stripped before any of it reaches a session.

Most specific wins: a repo entry, then `defaults`, then the top level, then the built-ins.
The resolver never fails on a bad config; it returns `warnings` and lets the hub say what is
wrong.

Nothing about the machine is hardcoded. Each of these is a command template whose
placeholders are substituted **already shell-quoted** — so do not put quotes around them:

| key | placeholders | default |
| --- | --- | --- |
| `terminal.spawn` | `{cwd}` `{title}` `{command}` | iTerm2 |
| `terminal.focus` | `{pid}` `{tty}` `{title}` | iTerm2 |
| `terminal.close` | `{pid}` `{tty}` `{title}` | iTerm2 |
| | | *`false` closes no tabs: `adjutant close` then exits 1 and clears nothing* |
| `terminal.title` | `{title}` | OSC escape written to this process's tty |
| | | *also names every tab `spawn` opens* |
| `wake` | `{pid}` `{tty}` `{subject}` `{line}` | iTerm2 `write text` into that session |
| `hubWake` / `workerWake` | the same | override `wake` for one direction |
| `agentRunner` | `{sessionId}` `{prompt}` `{worktree}` `{title}` | `claude --session-id {sessionId} --permission-mode auto {prompt}` |
| `hubRunner` | `{name}` `{sessionId}` `{prompt}` | `claude -n {name} --session-id {sessionId} --permission-mode auto {prompt}` |
| `agentResumeRunner` | same as `agentRunner` | `claude --resume {sessionId} --permission-mode auto {prompt}` |
| `hubResumeRunner` | same as `hubRunner` | `claude -n {name} --resume {sessionId} --permission-mode auto {prompt}` |
| | | *drop `{name}` and the session is nameless in every listing* |
| `notification` | `{title}` `{message}` `{nwo}` | `terminal-notifier` if installed, else `osascript` |
| `ide` | `{worktree}` | none — the procedures ask rather than guess |
| `worktreePattern` | `{repo}` `{branch}` `{name}` | `.claude/worktrees/{name}` |
| `startupDashboard` | — (`true` / `false`) | `true` |
| `hubAutoResumeHours` | — (a number, `0` to turn it off) | `3` |
| | | *`false` skips the listing a hub collects at startup; asking for one still collects* |

Omitting a key gets the built-in; setting it to `false` turns the behaviour off, which is a
different answer. `terminal` and the `wake` family merge key by key, so a repository can
change one half without restating the other. A setting of the wrong type is dropped *and*
reported in `warnings` — `adj config` is where to look when something silently does nothing.

A command line the terminal would have to type is **staged in a file once it grows past
about 900 characters**, and what gets typed is `sh /tmp/adjutant-spawn-….sh`. Long lines do
not fail, they arrive *corrupted* — a chunk dropped somewhere in the middle — and what runs
is whatever that mangling happened to spell. An `agentEnv` carrying a `PATH` is the ordinary
way to reach that length. A dry run is never staged: it is read by a person, and a path to a
file tells them nothing about what would have run.

**`ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` are forwarded onto the command line** of every
tab `hub --tab` and `work` open, when this process was given them. `ADJUTANT_HUB` already
travels as a flag; these two have none, and losing them does not fail — it splits. The tab
reads the default config and the default state directory, so the worker it starts registers
in one world while the hub that dispatched it waits in another, and both halves look healthy
from where they stand.

`{pid}` and `{tty}` are the operating system's names for a session — a process id, and the
terminal device it sits on (`ttys004`) — not a terminal's own id for a pane or a window. A
`focus`, `close` or `wake` template has to look that handle up before it acts: handing `{pid}` to
something expecting a pane id addresses a different number space and lands on whichever pane
happens to hold that number, so point those keys at a wrapper that does the lookup. `close`
is checked rather than believed for the same reason — a template is judged by its exit status
alone, so the worker's record is cleared only once that worker is actually gone, and
`adjutant close` exits 1 when it is still there.

The built-in `notification` prefers [`terminal-notifier`](https://github.com/julienXX/terminal-notifier)
and falls back to `osascript`, and the order is not a taste: macOS credits a notification posted by
command-line `osascript` to **Script Editor**, so the banner arrives from an app nobody asked for and
clicking it opens an empty Script Editor rather than the session that wanted you. With
`brew install terminal-notifier` on the machine the built-in becomes —

```jsonc
"terminal-notifier -title {title} -message {message} -sound Glass -activate com.googlecode.iterm2"
```

— a click that raises the terminal. `{nwo}` is there to go one better: it is the repository the
message is about, the same value `--repo` takes — `owner/name`, or the checkout's own directory
name when it has no usable remote — so a notifier that runs
`adj focus --repo {nwo}` on click lands on *that repository's hub tab*. Point the key at a script
rather than nesting a quoted command in the template, for the quoting reason the `wake` note below
gives. `{nwo}` is empty when `adjutant notify` runs outside a repository, and says so on stderr
rather than silently.

There is no built-in off macOS, where a missing notifier is silence rather than an error, so a Linux
hub needs the key set (`"notify-send {title} {message}"`). And when something already watches the
sessions — proctor's sidebar, a tmux status line — `false` is the honest answer rather than a second
banner. `adjutant notify --message … --dry-run` prints the command a template resolves to without
sending anything.

`wake` splits along the line the rest of the config does not: **how** to poke a session is a
property of the terminal, and **what to say** once poked is a property of the agent. So
`hubWake` / `workerWake` take a long form that overrides either half —

```jsonc
"wake": "wake-tab {tty} {line}",                  // the machine
"workerWake": { "line": "check `adj outbox`" }     // this agent has no MCP
```

— which is what a repo whose hub is one agent and whose workers are another needs. The
built-in sentences name MCP tools (`adjutant_pending`, `adjutant_outbox`), and each direction
is pointed at its own box. For anything longer than one command, point at a script rather than
wrapping it in `sh -c '…'`: a substituted value arrives with its own quoting and would end
the wrapper's quoted string early. A `spawn` template containing `{cwd}` is trusted to change directory
itself; one without gets a `cd` prepended. A new tab is named by the shell inside it calling
`adjutant title`, not through the terminal's own API — `set name of session` is the one
mechanism that does not generalise, since a profile whose title format is driven by user
variables ignores it and the tab silently keeps the wrong name. `agentEnv` is an object of environment variables
both the hub and its workers are started with, for a repository that runs under a separate
agent profile.

## The board

`adjutant serve` puts this repository's work on one page in a browser: what is in the
backlog, what has been handed to the hub, which worktrees have a worker in them, and what is
sitting unread in the inbox.

It is not a second coordination system. Every button on it ends in something this binary
could already do — handing a task over writes a `request` into the hub's inbox and pokes its
tab, through the same code `adjutant send` runs, so waking and notifying cannot drift between
the two callers. The page says so out loud: a strip along the bottom prints the command each
action maps to.

**It holds no clock.** Nothing polls a tracker and nothing wakes on a timer; a request
arrives because a person clicked. The page asks for state every two seconds, which is the
only repeating thing anywhere in it.

A task is a file in `~/.local/state/adjutant/tasks/<slug>/`, and it is deliberately not the
message that announces it: the message is read once and acked, and after that the hub would
have no way to say what became of the thing. The hub writes back to the record — `adjutant
task update --id … --status dispatched --worktree …` — and that is what the board shows.
The record is also the referee: a card dragged back to the backlog sets `status` there, and
the hub reads it once more just before it starts, so a task pulled back while its message
was still in the inbox does not get picked up anyway.

### Gates

A gate is the other half: something an agent has prepared for a person to look at, and the
ball handed over with it. A worker used to stop at three places — its plan, its diff, the
handover for a manual check — and ask in a tab nobody was watching. Now it writes the
question down, ends its turn, and the board shows it.

The payload is three frames rather than one wall of prose, because a reviewer who has to
read four hundred lines to find the two decisions that matter is a reviewer who approves
without reading: **what to look at**, **what was already decided** (folded away), and
**where the agent's confidence ran out**. A gate may also carry two designs side by side and
ask which one — the thing a terminal cannot do, since in a tab the second option has
scrolled past the first by the time you have read it.

The answer goes back out through that worktree's outbox and pokes the worker, which is
`adjutant tell` and nothing new. That path already survives the worker having died: the
answer simply waits there for whoever starts one next.

**A gate is one question, one decision and one comment.** Past two rounds it has become a
conversation, and a conversation is faster in the tab than through an outbox — so the board
counts the rounds, says so, and offers a button that raises the tab *without* closing the
gate. Leaving is not failing.

`adj gate open` reads its payload as JSON on stdin and answers with `server: up` or
`server: down`. That second answer is the whole reason it reports rather than just
succeeding: with nothing serving, a gate is a message into a directory no one opens, so the
procedure falls back to asking in its own tab. **A worker must never wait on a queue nobody
is watching.**

**The port is bound on `127.0.0.1` and everything needs a token**, kept in
`~/.local/state/adjutant/dashboard-token` and handed out in the URL the command prints.
Anything that changes state needs it in a header as well, and needs an `Origin` naming this
server — a page on another site can submit a form to a loopback port, but it cannot set that
header, and these endpoints are how work gets started.

## How the two sides reach each other

The address is the hub name, and both sides derive it the same way (`adjutant hub-name`)
rather than each spelling the rule out. It is `owner/name` collapsed to something readable
plus a digest of the lower-cased original — the readable half alone is not unique
(`acme/foo-bar`, `acme/foo_bar` and `acme-foo/bar` collapse together), and two repositories
sharing an address share an inbox. Lower-cased because the config finds a repository
case-insensitively, and an address that did not would split one repository in two. **Do not assemble the name by hand**; one character off is a
different box, and the digest is not guessable.

**Worker → hub** is a file. `adjutant send` (or the `adjutant_send` tool) writes into
`~/.local/state/adjutant/inbox/<slug>/`, and `adjutant pending` reads it. Delivery never
fails for want of a listener: if the hub is not running the message simply waits, and the
reply says which of the two happened. `adjutant hub-stop`, and a `kill -0` plus a command-line
check against the recorded PID, keep an exited hub from looking present. Every message also
carries a `worktree:` header — the absolute path the sender was standing in, derived rather
than typed — so a hub acting on one has an address it did not have to take from the body.
It is left out when git cannot place the sender in a worktree, and messages written before
it existed still read.

**Hub → worker** is also a file, but addressed by worktree rather than by session:
`adjutant tell` appends a `##` section to `{worktree}/.claude/adjutant-outbox.md`, and
`adjutant outbox` reads it. The worker's own record sits beside it, written by
`adjutant worker` — the launcher the tab actually runs, which records its PID and then
`exec`s the agent over itself, exactly as `adjutant hub` does for the hub. That record is
what makes `workerWake` possible; without a PID there is nothing to poke.

A worker's command line says nothing distinctive (it is whatever agent the config names), so
its record is anchored to the process start time instead — `exec` preserves it, which is what
lets a record written by the launcher still identify the agent that replaced it.

A file appearing in a directory notifies nobody, so delivery has a second half in both
directions: when the other side is up, `send` runs **`hubWake`** and `tell` runs
**`workerWake`** — by default iTerm2's `write text` into the session on that tty, which
reaches an interactive agent exactly as if the person had typed it. The line
typed points at the inbox rather than repeating the report, so the text lives in one place.
`send` fires `notification` either way; `tell` fires it only when the worker could not be
woken, since a woken worker reads the message without anyone's help. Both replies say which
of `present` / `woken` happened. Waking is best-effort by construction: the message is already delivered before the
hook runs, so a failed poke never fails a send, and the hub re-reads its inbox at three fixed
points anyway.

The point of routing all of that through templates is that the channel then works from any
agent in any terminal. The previous version was built on one coding agent's session list and
its session-to-session messages, and could not run anywhere else.

## Layout

```
repo  config  template  prompts        leaves — stdlib and their own input, nothing else
terminal  runner  notify  ide  messaging   may reach down, never sideways
cmd/  mcp                                  the only layer that joins them up
```

`scripts/check-layering.sh` enforces the arrows, and CI runs it. It is a fan rather than a
stack, so splitting it into a Cargo workspace would grow an empty relay layer between the two
real ones; as long as the check passes, splitting later stays a mechanical move.

## Development

```bash
cargo test                    # unit + end-to-end
./scripts/check-layering.sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

The tests are hermetic: `ADJUTANT_CONFIG` and `ADJUTANT_STATE_DIR` point into tempdirs, so no
test can read your config or drop a fixture report into a hub you actually have running.
Anything that would open a window, start an agent or notify a human runs under `--dry-run`.

## Subagents

Workers make use of specialized subagents during a task if your environment defines them:

- **Research**: An agent specialized in exploring the codebase and reading issues/docs (e.g. `task-researcher` or an agent whose description mentions research/exploration). If none is available, the worker falls back to the built-in `Explore` agent or `general-purpose` with explicit read-only instructions.
- **Review triage**: An agent that fetches and groups PR review comments (e.g. `review-triage` or triage-focused). Falls back to `general-purpose`.
- **Self-review**: An agent dedicated to independent diff verification (e.g. `self-reviewer` or review-focused). Falls back to `general-purpose` (or codex when configured).

Custom subagents are defined on the agent client side (such as `~/.claude/agents/*.md` in Claude Code). You do not need to define them to use `agent-adjutant`; a clean environment with only `general-purpose` works out of the box.

## Optional neighbours

[`proctor`](https://github.com/syarihu/agent-proctor) (worktree conventions, session ledger, tab colours) and [`lk`](https://github.com/syarihu/local-knowledge-cli) (the local knowledge
base) are used when they are on PATH and skipped when they are not.

Neither is needed. A convention tool does not *create* worktrees — it answers where they go
and what branch they sit on, and the `git worktree add` is typed either way — so what is
missing without one is a convention, which `adjutant worktree-path --name` supplies: branch
`{user}/{name}`, path `<main>/.claude/worktrees/{name}`, deliberately the same shape a
convention tool uses rather than a second competing one. The one thing with no equivalent is
its per-repo `copyFiles`; that belongs in this config's `postCreate`, which runs in both
cases.

[`terminal-notifier`](https://github.com/julienXX/terminal-notifier) is the third of these, and
the only one a built-in reaches for by itself: with it installed a notification comes from a
notifier whose click can be aimed, without it from `osascript` and therefore from Script Editor.

## License

MIT — see [LICENSE](LICENSE).
