# Review checklist

Extra things to look for when reviewing a change to agent-adjutant. It is written for the
self-review loop (either engine), but a person reviewing a PR can use it the same way.

This is an addition to the usual review, not a replacement. Every round still checks correctness,
the task's acceptance criteria, compatibility, tests and whatever else the change calls for; a diff
that passes every item here can still be wrong. The checklist only names the kinds of defect that
the usual review has kept missing in this repository.

The list comes from the review bots' history on this repository: 316 inline findings from
CodeRabbit and Copilot on PRs up to #332, each checked against the commits that followed it.
273 were fixed and 36 were left as they were. Every item below is a kind of defect that was raised
several times and fixed nearly every time — something a self-review could have caught before the
bots did. The PR numbers are examples, not the full list. Items 11 to 18 are the exception: they
come from the rules in `docs/architecture.md`, and their examples are issues an audit against those
rules filed.

## How to use it

- Check the items in "Every change" on every diff, and the conditional sections only when the diff
  touches what they name.
- A finding keeps the self-review loop's severity (see "Severity vocabulary" in
  `commands/adj-worker.md`): **must** when merging it would be a defect, **want** when it is correct
  but could be better, **scope** when it is a change the task did not ask for.
- Read "Do not raise" before reporting. Those were raised before and turned down on purpose; raising
  them again only keeps the loop from converging.

## Every change

### 1. Identify things by a key that is unique and still current

Do not find a task, worker, session, gate or PR by its worktree, path, basename, number or a
substring of its name. Two of them can share any of those. Match on the identifier the record
carries, and normalise paths the way git names them before comparing.

When what a result was for can change while it is on its way (the user switches to another task or
board, a task gets another PR), check that the result is not applied to the new one.

Examples: inferring a session's task from its worktree (#150, #154, #187), matching a Project v2
item by `.content.number` alone (#35), matching a worktree whose branch merely contains the key
(#25), finding the main checkout by path instead of `isMain` (#50), applying findings after a task
switch (#103), findings kept after the task's PR changed (#105), a gate comment sent to a different
gate (#145).

### 2. Do not read a failure as an answer

"Could not read", "failed" and "empty" are not "absent", "zero" or "done". When a record cannot be
parsed, a command fails or a lookup returns nothing usable, stop or report it; do not fall through to
the branch for the normal case. Where safety depends on the answer (stopping a process, removing a
worktree, closing a task), fail closed.

Examples: an unreadable worker record treated as no record (#16), a dangling symlink read as a hub
that is gone (#23), a failed `rev-list --count` reported as zero unpushed commits (#179), polling
errors dropped from the combined board state (#232), a failed hub delivery losing the message (#100),
a failed save of a task override ignored (#150), linked checkouts that cannot be listed (#168).

### 3. Reject bad input, and do it before any side effect

An unknown value should be an error, not a silent fallback to the default. A blank string or `null`
should mean the same as absent wherever "absent" has a meaning. Validate before claiming an ID,
writing a record or reading an issue, so a refusal leaves nothing behind. When the same value is
checked in two places, the two checks must agree.

Examples: an unknown `--agent` falling back to auto-detection (#42), worktree names git cannot branch
on and issue URLs without a host or with a bad port (#46), a blank `worktree` not treated as absent
(#62, #144), validating after an ID was claimed (#46, #65, #223), a parent check that accepts URLs
`check_parent` refuses (#225), a future `captured_at` trusted (#135).

### 4. Escape what goes into HTML, a shell or a template

Any value that reaches `innerHTML`, an inline `onclick`, a shell command or a runner template must be
escaped for that context, or passed in a way that needs no escaping (a `data-*` attribute with a
listener, a file or stdin instead of an argument, a quoted heredoc). This includes values written
into a command inside a procedure in `commands/`.

Examples: XSS in the board UI (#39, #67, #76, #77, #78), shell injection through a task title or
branch name in a procedure (#46, #102, #180), a ref interpolated inside single quotes (#12), quoted
assignments in runner templates (#39, #150), an unquoted path in the Makefile (#199).

### 5. Keep the parent's environment out of child processes

When starting git, an agent or a test binary, check which inherited variables can change the result.
`GIT_DIR`, `GIT_WORK_TREE` and `GIT_COMMON_DIR` point git at another repository when the parent was a
git hook. `ADJUTANT_*` variables point a child at the wrong hub.

Run git through `git` / `git_until` in `src/infra/git.rs` rather than building the command yourself.
Both drop the three repository-location variables (`REPOSITORY_LOCATION_ENV`). `git_until`, which
looks at a worktree that is not ours, also drops `GIT_INDEX_FILE` and the object-location variables;
plain `git` keeps them on purpose, so do not report that as a leak.

Examples: Git location variables (#44, #123, #179 — raised three times before it was fixed),
`ADJUTANT_HUB` inherited by a newly started agent (#16), tests inheriting the hub's environment
(#38).

### 6. Update a record as one step

A load → change → save of a record on disk must hold a lock for the whole sequence, and the write must
replace the file through a rename so a reader never sees half of it. When an operation saves several
records or shifts several items, a failure part way through must not leave some changed and some
not. Keep slow work (waking the hub, notifications) outside the lock.

Examples: appending a gate answer without a lock (#62), writing a gate in place (#62), a refresh that
aborts after saving some records (#70), a queue shift that fails half way (#81), creating a Jules
session twice from two starts (#99), the lock held across a wake (#100), a restart claim released
before the resumed hub is up (#221).

### 7. Keep the descriptions in step with the code

When behaviour changes, check `README.md` and `README.ja.md`, the `--help` text from clap, the MCP
tool descriptions and `config.example.json`. Inserting a clap field between a doc comment and its
field moves the help text to the wrong option.

Examples: README rows that no longer match the code (#41, #232), help text taken by a new `--hub`
field (#16), an MCP `kind` description missing kinds that `wake` names (#142), a README example
whose `--base` overrides the task's saved base (#133).

## When the change touches a procedure in `commands/`

### 8. A rule must hold on every route that reaches it

A rule added to one step is often reached from another menu item, another entry route, or another
source type. Check that it holds for each of them, and that no other paragraph of the same file still
says the opposite.

- Other entry routes: a rule scoped to menu item 2 that menu item 3 also passes through (#12), a
  parent-task route that skips the selection step whose rules it needs (#25).
- Other source types: a step that assumes GitHub issues when the source is `github-project`, Jira or
  Linear — keys, statuses, branch names and item IDs come from different places (#25, #35).
- Contradictions: an older paragraph that still says to omit a value the new step requires (#46).
- Required fields and order: a report missing a field the next step reads (#15), a phase set before
  the event it marks (#145, #172), a check placed after the action it should guard (#4).

## When the change touches the board UI (`src/ui/`)

### 9. Input, focus and redraws

- Enter pressed to confirm IME composition must not submit (#81, #82, #97).
- A redraw while a text box has focus must not throw away what is being typed; hold it and flush it
  when focus leaves (#67, #93, #145).
- A button whose request is in flight must not send it again (#103, #145), and a failed request must
  keep the user's draft (#145).
- `decodeURIComponent` and other parsing of the URL must survive malformed input (#67).

### 10. Accessibility

- Anything clickable is a `<button>` (or has a keyboard path), and icon-only controls have an
  accessible name; decorative icons are `aria-hidden` (#76, #78, #145, #184).
- Tabs are wired as tabs: `aria-selected`, `aria-controls` and the matching panel (#67, #77, #210).
- Keyboard focus stays visible, including on visually hidden inputs (#78).
- Text and badges keep their contrast in dark mode (#75).

## When the change adds or moves Rust code under `src/`

These items come from the rules in [architecture.md](architecture.md#rules-and-what-enforces-them),
not from the bots' history: they are the parts of those rules that only review keeps (item 17 also
has a CI job, which is not a required check). The rule has the details.

### 11. One operation per file, in the module that owns it

A new operation is a file named after what it does, in the lowest module that holds every record it
writes. Shared helpers go in `model.rs`, `store.rs` or the owning operation, not a new catch-all
file (rule 2 and "Where does my change go?"). `check-layering.sh` refuses only `usecase.rs`,
`util.rs` and `common.rs`.

Examples: a new operation added to another operation's file, or code only one operation uses kept
in `model.rs` (#561), a rule copied into a second module instead of written once and called (#562).

### 12. No other module's store

Code outside `registry`, `mail`, `task` and `gate` does not build their paths or take their locks;
it calls their operations (rule 3). Nothing checks that `mod store;` stays private, and the
compiler cannot see a path built by hand.

### 13. Reads take a state root and a hub slug

A new read takes the state root and a hub slug so the board can read every hub; a write takes a
`Context` (rule 4).

Examples: a read that starts writing (a seen marker) but keeps its `(root, slug)` signature, and a
read that takes a `Context` (#568).

### 14. Operations return values, transports word them

An operation returns a typed outcome, never text for stdout. Words two transports share go in
`transport::wording`, and a rule such as when to wake stays in the operation (rule 5).

Examples: an operation that prints a warning with `eprintln!` instead of returning it (#565), a
sentence written separately in the CLI and MCP instead of in `transport::wording` (#563), a
transport that decides when to wake, what to write or which source wins instead of calling one
operation (#564).

### 15. Undo through the other modules' operations

An operation that writes to several modules takes a failed step back with their own operations,
latest first, never by writing their stores (rule 6).

### 16. No traits for stores

Stores are built from the state dir in `Context`. Code that runs processes gets a `_with` variant
that takes the runner, not a trait (rule 7).

Examples: new code that runs `ps`, git, tmux or a shell with no `_with` variant or runner argument,
a `_with` that ignores the table it is given (#567).

### 17. A move is its own PR

A diff that moves items and also changes behaviour is split (rule 9). A move-only PR carries the
`move-only` label, so CI runs `check-move-only.sh` on it.

### 18. Records stay readable by the old binary

A new key in a stored record is optional to the new binary, unknown keys are kept, and paths, key
names, lock names and file names stay as they are (rule 10). The unit tests cover only keeping
unknown keys.

Examples: a stored struct read and written back without a catch-all for unknown keys (#566), a new
path under the state dir missing from [Where state lives](architecture.md#where-state-lives) (#569).

## Do not raise

These were raised by the bots and left as they are on purpose. Do not report them unless the change
in front of you makes them newly reachable. The list covers only these kinds; it does not hold back
anything else the usual review finds.

- **Strict mutual exclusion between board clients or between processes starting up.** For example,
  making queue insertion atomic across browser tabs (#81) or claiming the dashboard record before the
  board starts (#91). Races inside one process's record update are still in scope (item 6).
- **Hardening `close` against PID reuse or workers started by hand**, such as a worker with no
  record, or a PID recycled between two checks (#3).
- **Collision-proof ID generation**, such as two worktrees with the same basename producing the same
  session ID, or a 32-bit digest collision (#150, #152). Looking a record up by a basename or a
  substring is still a defect under item 1; this is only about how new IDs are made.
- **Touch target sizes** (#76, #78, #217).
- **Limits of the shell parsers in `scripts/`** that are already written down in the script, such as
  `check-move-only.sh` not following block comments in its line-by-line stage or token trees inside
  macros (#278, #316).
- **Adding tests only for coverage** (#226, #275, #332). A regression test for the behaviour a change
  fixes is still worth asking for, as a want.
- **The known exceptions to rule 3 in `docs/architecture.md`** (from that page, not the bots): the
  board's session clean-up (`board/session/clean_up.rs`) and `kernel::worktree_state` naming
  `.claude/adjutant-*` and `task-brief.md` in a worktree, and
  `#[cfg(test)] pub(crate) use store::...` for other modules' test fixtures.
