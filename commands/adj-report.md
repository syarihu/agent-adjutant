---
description: Hand a bug found outside the current task to the resident hub, without leaving the worktree
---

Report — hand **a bug that is not part of your current task** to the hub (the resident `adj-hub`
session) without leaving the worktree you are in. The hub takes over filing it and deciding whether
to start it, so once it is handed over you go back to your own work.

Argument `$ARGUMENTS` = a description of what you found. If it is empty, put it together from the
conversation so far.

Talk to the user in the language they use with you (or the one your agent is set to). A quoted line
in this procedure says what to tell them, not the words to use.

## Three things this rests on

- **Do not fix it yourself.** An unrelated fix mixed into this task's diff can be neither reviewed
  nor reverted.
- **Do not file it yourself either.** Duplicate checks, templates, labels, board registration and
  the decision to start are kept in one place, on the hub's side.
- **Do not wait for a reply.** Send it and you are done. The hub's reply arrives later.

## Context — gather once, first

- `adjutant_hub_status` (no arguments; it resolves the repository from the worktree you are in).
  It returns `repo` / `main` / `hubName` / `present` (whether a hub is running) / `waiting` (how
  many messages are in its inbox). **Do not build the hub name yourself.** The name matches because
  the side that claims it and the side that looks for it go through the same derivation; a copy of
  the rule drifts the moment it is made.
- `git rev-parse --show-toplevel` and `git branch --show-current`
- The first 12 lines of `.claude/task-brief.md`, if it exists

## Steps

### 1. Pin down the facts

From `$ARGUMENTS` and the conversation so far, settle the symptom and where it is. **Do not write a
file:line from memory or a guess** — open the one or two places and check. The hub is looking at a
different branch, so a miss here is one nobody can fix.

Also ask yourself once, here, whether it is within the current task. If it is, this is not a
report; just fix it.

### 2. Put the body together

```
## Symptom
## How it was found / steps to reproduce
## Location          file:line (more than one is fine)
## Suspected cause   Only as far as you know. If you do not know, "not investigated"
## Why it is outside the current task
## Found in          {task_id} {task_url}
## Parent task       {parent_task} (`-` if there is none)
## Reported from     {repo} / {branch} / this session's name
```

- Found in comes from the "Task" line of `.claude/task-brief.md`. If there is none, take the key
  from the branch name (`{user}/ALPHA-957` → `ALPHA-957`). **It is not the brief's "Parent task"
  line** — what goes here is the task you are holding now, because that is the work in which the
  bug turned up. The same holds when you were handed a subtask: re-pointing it upwards loses the
  context it was found in.
- Parent task is the brief's "Parent task" line, copied as it stands. If there is none, `-`.
  **Do not substitute Found in** — the hub decides whether the filed issue stands alone or becomes
  a sub-issue against this line, so handing it only the subtask hangs a bug that belongs beside
  that subtask underneath it.
- **30 lines at most. Do not paste an investigation log.** The hub repacks it into the issue
  template, so raw material is enough here.

### 3. Show it to the user and get a go-ahead

**Text written in the same turn as `AskUserQuestion` does not reach the user's screen.** Put the
body into the `preview` of Q1's "Send as is" option, so one call shows it while asking. If the body
is too long to fit in `preview`, that is a sign it runs past 30 lines — cut it. Only if it is still
long, print the body as plain text, **end the turn**, and ask in the next one.

`AskUserQuestion` (two questions in one call):

- **Q1 "Send this as is?"** — Send as is / I want to change it (the change goes in Other)
- **Q2 "How far should the hub go?"** — **File only** (put it on the board) / **File and start
  now** (the hub cuts a worktree and starts a worker in another tab)

Append Q2's answer to the end of the body as `## Scope`. **Asking here is the point**: there may be
nobody at the hub's tab. If the hub had to ask, the report would sit there until the user came to
that tab.

**Leave this line out and it falls to "file only"** (the hub's default; it never starts on its own).
Forget it on a request that wanted an immediate start, and the bug is filed and nothing more.

### 4. Hand it over

Call `adjutant_send` once.

| Argument | Content |
| --- | --- |
| `body` | The body from §2 |
| `subject` | One sentence with the conclusion. It is the only line a person sees in the list |
| `from` | This session's name (the worktree name if there is none) |
| `kind` | `report` |

An example `subject`:

```
Search result images are squashed vertically (found while working on ALPHA-957; file and start now)
```

**Do not name a recipient.** The arguments have no recipient on purpose: it reaches the hub of the
repository you are in. A bug in another repository is reported from a checkout of that repository
(the `repo` argument can send to another hub, but a hub can only read its own checkout, so it is
of little use).

**It does not fail when no hub is running.** It is delivered to the hub's inbox directory, which the
hub reads on startup and before going back to waiting. If a hub is running, it is also woken up
(how is up to the config). The `present` / `woken` you get back:

- `present: true, woken: true` → a running hub was woken. It will be dealt with.
- `present: true, woken: false` → it is in the inbox. The hub reads it the next time it looks.
- `present: false` → it was left as a note. Tell the user "No hub is running, so I left it in its
  inbox. If it is urgent, start one with `adjutant hub`." To help start one:
  `adjutant spawn --cwd '{toplevel}' --title 'start hub' -- adjutant hub`
  (`{toplevel}` is the absolute path from Context. **Do not leave it out** — a new tab's cwd
  depends on the terminal profile, and outside a git repository it fails at once).

**Do not decide against sending because of the hub's state.** It is delivered to a file, so nothing
is lost whether the hub is waiting on a question or busy. Sending first and telling afterwards is
the right order.

### 5. Go back to your own work

Once sent, this procedure is over. The hub's reply arrives later; when it does, **note the issue
number in one line and go back to your task**. No reply is needed. Do not get drawn into discussing
what was filed — that is between the hub and the user.

**There is one exception: a reply whose first line starts with `[question`, followed by an identifier
and `]` (`[question {YYYYMMDD-HHMMSS}]`).** A hub started before its procedures were in English writes
`[質問 {YYYYMMDD-HHMMSS}]` instead; treat that the same. The hub is asking for
something it needs in order to file, so **answer once**. The hub is looking at a different branch and
cannot read the code in your worktree — only you can. Check the file:line, reply briefly, and leave
it there (no discussion). Reply with `adjutant_send` as well, with `kind` `answer` and the original
question's identifier at the start of `subject`.

A reply starting with `[ack]` only says it was received; send nothing back.

## Do not

- **Do not have the hub do something your own permissions refused** (permission laundering). Raise
  what was refused with the user.
- Poll or loop on `sleep` waiting for the hub's reply.
