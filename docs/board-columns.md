# Board columns

A design for letting each team lay out the board's columns its own way without losing what the
board does today: cards move on their own, because every column is tied to something adjutant
knows about a task. It is for the people and agents who will build it. The sections follow the
questions in [#559](https://github.com/syarihu/agent-adjutant/issues/559); the last one splits the
work into issues.

The idea in one line: adjutant owns a fixed vocabulary of **facts** about a card, and a **column**
is a label plus a condition over those facts. A layout never names code, only facts, so any layout
keeps moving cards by itself, and the worker procedure goes on reporting facts (`adj phase`, `adj
task update`), never columns.

## Where a card's column comes from today

The board has two halves, and a task can be on both at once.

**The agent board** has six columns, defined only in the page (`AGENT_COLUMNS` in
`src/ui/core.js`): `before` 着手前, `plan` 計画, `implement` 実装, `selfreview` セルフレビュー,
`pr` PR・レビュー対応, `done` 完了. The page decides the column itself, in `agentColOf`:

1. No task, or status `backlog` / `queued` → `before`. Status `done` / `cancelled` → `done`.
2. A Jules task (`executor` is `jules`, or it has a `julesSession`): a PR or Jules state
   `COMPLETED` → `pr`; `QUEUED` / `PLANNING` / `AWAITING_PLAN_APPROVAL` → `plan`; `IN_PROGRESS` /
   `AWAITING_USER_FEEDBACK` / `PAUSED` / `FAILED` → `implement`; otherwise an open `plan` gate →
   `plan`, an open `diff` or `verify` gate → `selfreview`, else `plan`.
3. A worker row with a known phase → `AGENT_COL_OF_PHASE`: `plan` → `plan`, `implement` →
   `implement`, `self-review` and `verify` → `selfreview`, `pr`, `pr-bots`, `review` and `report` →
   `pr`.
4. No phase: an open `plan` gate → `plan`; an open `diff` or `verify` gate → `selfreview`.
5. Status `pr` or a PR → `pr`; otherwise `plan`.

A worker with no card (a bare session) goes by `AGENT_COL_OF_PHASE`, or `implement` when it has no
phase (`src/ui/board.js`). None of this has a test: [#466](https://github.com/syarihu/agent-adjutant/issues/466)
left it in the page because there was no rule on the server to share.

**The human board** has six columns: `dispatch` 着手確認, `plan` 計画の承認, `diff` 差分レビュー,
`verify` 動作確認, `prreview` PRレビュー, `question` 質問. Since
[#465](https://github.com/syarihu/agent-adjutant/issues/465) the server decides the inputs and the
page joins them (`humanColOf`):

- Each open gate carries `humanCol`, from its kind (`HumanCol::of_gate` in
  `src/board/view/columns.rs`): `dispatch` and `issue` → `dispatch`, `plan` → `plan`, `diff` →
  `diff`, `verify` and `result` → `verify`, `relay` → `prreview`, `question` → `question`. Gates
  kept as records (`wait: false`) are not open and place nothing.
- Each card carries `waitsOnPerson` (`waits_on_person` in the same file, over
  `task::pr_waits_on_person` and the PR's `PrTurn`): whether the PR is the person's turn.
- The page takes the open gate's `humanCol`, else `prreview` when `waitsOnPerson`, else no human
  column. A parent's hub card takes `ownerHub.humanCol`, built the same way on the server.

The column is on the gate, not on the card, on purpose: when the person answers a gate, the page
drops it from its copy of the state at once (`mergeStates`), and the card leaves the column without
waiting for the next poll. `agentColOf` reads the open gate too, so the agent column moves at the
same moment.

Other things read the same inputs:

- Through `agentColOf`: the task panel's stepper (`STEPS` in `src/ui/task-panel.js`).
- Through `AGENT_COL_OF_PHASE`: the phase label on a session's row (`sessionRowHtml` in
  `src/ui/sessions.js`).
- Through `humanColOf` or `columnOf`: drag and drop (`ALLOWED`), the page's waiting counts
  (`waitingIn`, `renderCounts`), the panel's 「…を待っています」 pill, the card's waiting mark, the
  stuck strip, and the agent board's sort (cards waiting on the person go last).
- Gates with no card on the board are placed by `g.humanCol`.
- The server's sidebar counts (`board_counts`) read an open gate or `waitsOnPerson`.
- `FINISHED_PHASES` in the sessions view and the stuck badge (`stuckOf`) read the phase.

The session ledger's `waiting` changes no column; it marks the card and fills 要対応.

None of the board code reads the task's `kind` or `doneWhen`. A report-only task's worker sets phase
`report` as soon as it starts investigating, so an investigation sits in PR・レビュー対応 for its
whole run and then in 動作確認 with its `result` gate. This is the clearest case of a card in a
column that does not describe it.

## The facts

A fact is a named value adjutant derives from its own records, on the server, for every card. The
names and values are adjutant's: a release can add a fact or a value, and the layouts that do not
use it are unaffected. Each fact's value is one word, or absent.

| Fact | Values | Comes from |
|---|---|---|
| `status` | `backlog`, `queued`, `dispatched`, `pr`, `done`, `cancelled` | `task::Status` |
| `step` | `plan`, `implement`, `self-review`, `pr` | Derived (below) |
| `phase` | `plan`, `implement`, `self-review`, `verify`, `pr`, `pr-bots`, `review`, `report` | The worker row's phase (`registry::PHASES`) |
| `executor` | `worker`, `jules` | `task::Executor` (`jules` also when there is a `julesSession`) |
| `jules` | Jules's state, lower case (`queued`, `planning`, `in-progress`, `completed`, `failed`, …) | The Jules session, for a Jules task |
| `work` | `implement`, `investigate`, `design`, and any added in config | The task's new `work` field ([below](#what-kind-of-work-a-task-is)) |
| `doneWhen` | `report-only`, `verify`, `pr`, `review` | `task::DoneWhen` |
| `gate` | `plan`, `diff`, `verify`, `dispatch`, `issue`, `question`, `result`, `relay` | The kind of the task's open (waiting) gate |
| `prTurn` | `draft`, `unrequested`, `other-reviewer`, `checks`, `changes`, `merge`, `ci-failed`, `merged`, `closed` | `task::PrTurn` from the stored `prStatus` |
| `waitsOnPerson` | `yes`, absent | `waits_on_person`, unchanged |
| `session` | `idle`, `running`, `waiting`, `done`, `failed` | The ledger row of the task's worker session (a status this binary does not know reads as absent) |
| `parked` | A park reason (`pdm`, `design`, `review`, `merge-timing`, `other`, and any added in config) | The task's park ([#555](https://github.com/syarihu/agent-adjutant/issues/555)); absent once `status` is `done` or `cancelled` |

A phase this binary does not know reads as absent too, so a record written by a newer binary never
breaks a card. `session` needs the ledger, which the board's poll reads only for the sessions view
(`with_sessions`); it is read for the cards only when the board's layout names `session`, so the
`default` layout costs the poll nothing new.

`step` is the one derived fact: where work in progress stands, whoever does it. It is `plan`,
`implement`, `self-review` or `pr`, computed exactly as steps 2 to 5 of `agentColOf` above: from the
Jules state for Jules, from `phase` for a worker, from the open gate's kind when neither says, then
`pr` for status `pr` or a PR, and `plan` when nothing says anything. It is absent when the task is
not in progress (status `backlog`, `queued`, `done` or `cancelled`), so a column that names `step`
never catches a finished task. A bare session (a worker with no card) has no `status` and gets
`step` from its phase, or `implement` with no phase, as `bareCol` does today.

The task panel's stepper reads `status` and `step`, not a column, so it stays the same whatever the
layout: nothing lit before the task starts, every step lit once it is `done`, and `step` in
between.

For a parent's hub card, the `gate` fact is the open gate the page finds today (one on the card's
own board), not the hub's gate that `ownerHub` reads; changing that is a separate fix, not part of
this design.

Two things are deliberately not facts:

- **Whether the person has seen a card** ([#554](https://github.com/syarihu/agent-adjutant/issues/554)).
  It lives in each browser, so the server cannot place by it.
- **Stuck.** It depends on the clock, and a column that a card enters because time passed would
  move cards with nobody doing anything. It stays a badge, computed as today.

A fact names one task. A PR's facts are the task's PR (`task.pr`); a branch with several PRs is
[#30](https://github.com/syarihu/agent-adjutant/issues/30)'s question, not this one's. Facts for
other records (the review requests of 他人の PR,
[#234](https://github.com/syarihu/agent-adjutant/issues/234)) are out of scope: that board has its
own fixed columns over its own records.

The server sends the facts with each card (`facts` in `/api/state`), so every view reads the same
derivation instead of keeping its own.

## Columns as conditions

A layout is two ordered lists of columns, one per half of the board. A column has an `id`, a
`label`, an optional Material Symbols `icon` and `hint`, and `when`: the condition. The agent side
of today's board with an investigation column added looks like this:

```json
{
  "board": {
    "agentColumns": [
      { "id": "before",      "label": "着手前",           "when": { "status": ["backlog", "queued"] } },
      { "id": "investigate", "label": "調査",             "when": { "work": "investigate", "step": "*" } },
      { "id": "plan",        "label": "計画",             "fallback": true },
      { "id": "implement",   "label": "実装",             "when": { "step": "implement" } },
      { "id": "selfreview",  "label": "セルフレビュー",   "when": { "step": "self-review" } },
      { "id": "pr",          "label": "PR・レビュー対応", "when": { "step": "pr" } },
      { "id": "done",        "label": "完了",             "when": { "status": ["done", "cancelled"] } }
    ]
  }
}
```

- `when` is an object: every fact named must match (and). A fact's value is a word, a list of words
  (any of them), `null` (the fact is absent) or `"*"` (present, any value). `when` can also be a list
  of such objects, and then any one matching is enough (or). There is no negation; order does that
  job, as below.
- **The first matching column wins.** A card is in exactly one agent column, and in at most one
  human column.
- **The agent side has exactly one fallback column**, which has no `when` and takes every card that
  matches nothing. So no card disappears, whatever the layout. The human side has no fallback: a
  card that matches no human column is not shown there, as today.
- The page shows the columns in the order of the list, and the conditions are tried in the same
  order, with one exception: the fallback is tried last wherever it sits, so it can be shown where
  it belongs (計画 second). One order for both keeps a layout readable; a column that must not catch
  what a column shown after it should get says so in its condition (above, the working columns name
  `step`, which a finished task does not have, so 完了 last still gets it).

Today's human side, written the same way, is `dispatch` (`gate`: `dispatch` or `issue`), `plan`
(`gate`: `plan`), `diff` (`gate`: `diff`), `verify` (`gate`: `verify` or `result`), `prreview`
(`gate`: `relay`, or `gate` absent and `waitsOnPerson`), `question` (`gate`: `question`). The
`gate` absent in `prreview` keeps today's rule that an open gate outranks the PR: a card with a
`question` gate and a PR waiting on the person stays in 質問.

**Where it runs.** The server evaluates, in `board::view`. Each card gets `agentColumn` and
`humanColumn` computed **without** its open gate; each open gate gets `agentColumn` and
`humanColumn` computed from its card's facts **with** the gate. The page reads the open gate's
pair, else the card's, as `humanColOf` does now. When the page drops a gate it just answered, the
card falls straight to the columns it will have after the next poll, on both halves, as today. A
gate with no card on the board is evaluated with only its own `gate` fact. A parent's hub card gets
the same fields under `ownerHub`. A present worker with no card gets `agentColumn` on its worker
row, from its `phase`, `step` and `session`, and the sessions view labels a row's phase with that
column's label instead of `AGENT_COL_OF_PHASE`.

The gate's `humanCol` is replaced by `humanColumn`; `waitsOnPerson` stays, as a fact. The tests that
pin the old names (`the_page_places_a_pr_by_its_turn_and_says_when_the_poll_has_stopped`, the
gate's `humanCol` in `tests/board_state.rs`) move with them.

Shipping the layout to the page and evaluating there was the other option. It would put the
condition language in JavaScript next to the Rust one, which is what #466 removed, and the page has
no tests that run. Computing the columns twice per card is cheaper than a second evaluator.

The server also sends the layout itself (`layout: { agent: [...], human: [...] }`, ids, labels,
icons and hints), and the page renders the board from it. `AGENT_COLUMNS`, `HUMAN_COLUMNS` and
`AGENT_COL_OF_PHASE` leave the page.

**Ids are unique per side.** The default layout keeps today's ids, including `plan` on both sides,
because saved page state and links name them. Wherever a column is named outside its own board (a
label in the task panel, a stored filter), the page names it with its side (`agent:plan`,
`human:plan`), so the mix-up behind [#488](https://github.com/syarihu/agent-adjutant/issues/488)
(since closed) cannot come back with a layout that reuses an id.

**「すべて」** merges several boards in the page, and their layouts can differ. It asks each board
for its state with the `default` layout (`/api/state?layout=default`), and draws `default`'s
columns. A board's own page uses its own layout. The cross-repository view that replaces 「すべて」
([#553](https://github.com/syarihu/agent-adjutant/issues/553)) reads facts instead, as below.

## Presets and the config

The key is `board`, a machine setting like `stuckAfterMinutes` (a `SETTING_KEYS` entry, with an
"an object" shape in `accepted_shape`): read again on every poll, so a change shows without a
restart. Like most settings it is taken **whole** from the most specific level that has it (the
repository's entry, then `defaults`, then the top level), not merged key by key as `terminal` is. In
the example below, `acme/web` names its own `preset` because nothing comes over from `defaults`.
Each repository's board can have its own layout.

```json
{
  "defaults": { "board": { "preset": "investigation" } },
  "repos": {
    "acme/web": {
      "board": { "preset": "default", "humanColumns": [ /* … */ ] }
    }
  }
}
```

- `preset` names a built-in layout. `agentColumns` / `humanColumns` replace that side of the preset
  whole; the other side stays the preset's. There is no merging column by column: a list whose
  order is its meaning cannot be patched safely.
- `parkReasons` and `workTypes` add values to `parked` and `work` (`[{ "id": "legal", "label":
  "法務の確認待ち" }]`). Built-in values cannot be removed or relabelled.
- An empty config is the `default` preset, which is today's board exactly: the same twelve columns,
  ids, labels, order and placement.

Presets shipped:

| Preset | Agent side | Human side |
|---|---|---|
| `default` | Today's six | Today's six |
| `investigation` | Adds 調査 (`work: investigate`) and 設計 (`work: design`) after 着手前, so those tasks stay in one column whatever their phase | Today's six |
| `waiting` | `default`, plus 他の人の確認待ち (`parked: "*"`) right after 着手前 | `default`, plus 置いている (`parked: "*"`) first |

In `waiting`, the parked column comes before the gate columns on the human side, so a parked task
with an open gate shows in 置いている with its gate still open ([#555](https://github.com/syarihu/agent-adjutant/issues/555)
keeps the gate open and answerable).

**Mistakes show in `warnings`.** The resolver never fails, so a layout with an error is not used:
that side falls back to the preset (or to `default` when the preset itself is unknown) **as a
whole**, never half a layout, and a warning names the column and what is wrong. The checks:

- an unknown `preset`, fact, or value of a fact whose values adjutant owns (`work` and `parked`
  accept the configured additions);
- a column without `id` or `label`, or a duplicate `id` on the same side;
- on the agent side, no fallback or more than one; on the human side, any fallback;
- a `when` on the fallback, or a column other than the fallback with none;
- a column nothing can reach: one after a column whose `when` is empty (`{}`), and one whose
  `when` is the same as an earlier column's (the fallback is not counted, since it is tried
  last).

`kernel` (rank 2) cannot name `task::Status` or `gate::Kind`, so it only checks that `board` is an
object, like any other setting. The vocabulary check lives in `board::view` with the evaluator, and
`adj config` and `adjutant_config` append its warnings to the resolver's (from `transport`, which
may name `board`). The board also shows the
warnings above the layout it fell back to, so the person who wrote the layout sees why it is not
there.

## What kind of work a task is

The task gains `work`: `implement`, `investigate` or `design`, plus anything in `workTypes`. The
name `kind` is taken: `task::Kind` (`start`, `file-and-start`, `investigate`, `tell-worker`) is how
the task was handed over, and it only shapes the request text. The two stay separate: one says what
the person asked the hub to do, the other what the work is.

- An absent `work` reads as `investigate` when `doneWhen` is `report-only`, else `implement`. Old
  records need no change, and a binary that does not know `work` keeps it in `extra` (rule 10 in
  [architecture.md](architecture.md)).
- `adj task add --work <type>` sets it, and `adj task update --work <type>` changes it: a task that
  starts as an investigation can become an implementation. An unknown type is refused before
  anything is written. The check is one function in `task`, given the configured `workTypes`, and
  the layout check in `board::view` calls the same one, so the two cannot disagree.
- The hub passes `--work investigate` for an investigation-only request and `--work design` for a
  design task. The worker never sets it.
- `doneWhen` keeps its meaning (where the work stops). `work` does not change any procedure, only
  facts.

## Columns a person moves cards into

Some states adjutant cannot know: waiting on a product manager's confirmation, on a designer, on
the right moment to merge. [#555](https://github.com/syarihu/agent-adjutant/issues/555) parks a task
with a reason, on the task record, from the board or the CLI. **A manual column is a column whose
condition is a park reason.** There is no second mechanism: parking is the one fact a person sets by
hand, and a layout decides where parked cards show.

- **Into it**: dragging a card onto a column whose `when` names `parked` parks the task with that
  reason (a column with `"*"` asks for one). Parking from the task summary or with `adj task park`
  does the same.
- **Out of it**: dragging a parked card onto any column whose `when` does not name `parked`
  unparks it, and the card then goes where its other facts put it (which need not be the column it
  was dropped on). 置くのをやめる and `adj task unpark` do the same.
- **What stops a fact from pulling it out**: nothing moves a parked card but a person, as long as
  the parked column comes before the columns its other facts match. The `waiting` preset puts it
  first on the human side and right after 着手前 on the agent side. A layout can put it later; the
  warnings catch only the plain unreachable cases above, so the README says to keep parked columns
  early.
- **What ends it anyway**: the `parked` fact is absent once the task is `done` or `cancelled`, so
  a merged PR still takes the card to 完了, even when the binary that wrote `done` does not know
  about parking and left the park on the record. Clearing it when the status is written is only
  tidying.
- Any other drop is refused unless it sets a `status` a person may set (`backlog` / `queued`, as
  `ALLOWED` allows now).

## Counts, 新着 and the other views

The board's counts and the cross-repository 「いまの仕事」 view
([#549](https://github.com/syarihu/agent-adjutant/issues/549),
[#553](https://github.com/syarihu/agent-adjutant/issues/553)) read facts, not columns. Columns
differ from one repository to the next, and a layout must not change what "it is your turn" means.

- **Whose turn it is** stays independent of the layout: an open gate, or `waitsOnPerson`, and
  not `parked`. The server sends it on each card as `yourTurn` (and computes it the same way for
  `board_counts`). Everything listed earlier that reads `humanColOf` to ask "is this the person's"
  rather than "where does it go" (the page's counts, the 「…を待っています」 pill, the card's waiting
  mark, the stuck strip, the agent board's sort) reads `yourTurn` instead. A parked card in the
  `waiting` preset's 置いている column is shown on the human half but is not the person's turn and is
  not counted. The sidebar counts, the badges and 新着
  ([#554](https://github.com/syarihu/agent-adjutant/issues/554)) use it.
- #553's state boxes and the parent's progress segments (merged / PR / working / not started) are
  fixed derivations over `status`, `prTurn`, `step` and `session`, written once on the server next
  to the facts. They can show a card's column label from its own repository as text.
- [#557](https://github.com/syarihu/agent-adjutant/issues/557)'s notifications read `session` and
  `parked`; [#558](https://github.com/syarihu/agent-adjutant/issues/558)'s replacement for 要対応 reads
  `gate` and `session` (a session held by an open gate is not listed, as now).
- [#554](https://github.com/syarihu/agent-adjutant/issues/554) and
  [#556](https://github.com/syarihu/agent-adjutant/issues/556) need changes of facts, not only their
  current values. The facts are one struct per card, so comparing it with the previous poll's gives
  the change; which changes count as "your turn" is theirs to decide.
- `FINISHED_PHASES` in the sessions view reads `phase` and the stuck badge reads its own inputs;
  neither moves to columns.

## Migration

- No config change: the `default` preset is today's layout, with today's ids. The first
  implementation step moves `agentColOf` to the server with a table test of every branch listed
  above, then the page renders from what the server sends; the board must look the same before and
  after, and the tests say so.
- No record change: `work` and `parked` are new optional keys, and an absent one has a defined
  value.
- `the_page_has_a_word_for_every_value_the_server_sends` loses its checks of `AGENT_COL_OF_PHASE`
  and `HUMAN_COLUMNS`, which leave the page; its checks of `KINDS`, `PHASE_LABEL`,
  `DECISION_LABEL`, `PR_TURN` and `PR_TURN_WHY` stay. Rust tests take the place of the two that
  go: every `step`, `gate` and `PrTurn` value reaches the column of the `default` preset that the
  page used to give it, and the fact enums keep exhaustive matches, so a new value cannot be added
  without deciding where it goes.

## Where the new state lives

| What | Where | Owner |
|---|---|---|
| `work` | On the task record (`tasks/<slug>/<id>.json`), optional | `task` |
| `parked` (reason, text, since) | On the task record, optional ([#555](https://github.com/syarihu/agent-adjutant/issues/555)) | `task` |
| `board` setting | The config, repository > `defaults` > top level | `kernel` (shape), `board::view` (vocabulary) |
| Facts, the presets, the evaluator | Computed per poll, never stored | `board::view` (next to `columns.rs`) |

## Implementation split

In order. Each one is a sub-issue of #559 and lands on its own. Until the fourth, the columns stay
the same as today (the third adds only a pill on the card).

**1. The agent board's column is decided only in the page, with no tests**

```markdown
## What happens

Where a card sits on the agent board is computed in the page (`agentColOf` in `src/ui/core.js`),
with branches for Jules, the worker's phase and the open gate, and nothing tests it. Every other
view that wants the same answer (the task panel's stepper, the sessions view) has to read it from
the page or compute it again, and the server cannot give it to a view of its own.

## Proposal

Add the facts from `docs/board-columns.md` to `board::view`: one struct per card with `status`,
`step`, `phase`, `executor`, `jules`, `doneWhen`, `gate`, `prTurn`, `waitsOnPerson` and `session`,
sent as `facts` on each card in `/api/state`. `step` reproduces `agentColOf` branch by branch; port
its precedence into a table test, including bare sessions (on the worker row) and a parent's hub
card. The task panel's stepper reads `status` and `facts.step` (nothing lit before the task starts,
all lit once done). No change on the board.
```

**2. The board's columns are hard-coded in the page**

```markdown
## What happens

The two halves of the board are fixed lists in the page (`AGENT_COLUMNS`, `HUMAN_COLUMNS`,
`AGENT_COL_OF_PHASE`), so a different layout means changing the page, and the server cannot
check that every value it sends has a column.

## Proposal

Add the column evaluator and the `default` preset from `docs/board-columns.md` to `board::view`:
`agentColumn` and `humanColumn` on each card (without its open gate) and on each open gate (with
it), the same under `ownerHub`, `agentColumn` on a present worker with no card, `yourTurn` on each
card, and `layout` in the state. The gate's `humanCol` gives way to `humanColumn`. The page renders
both halves from `layout`, places cards by those fields, reads `yourTurn` wherever it asks whether
a card is the person's, labels a session's phase by its row's column, and names columns with their
side outside their own board; drop `AGENT_COLUMNS`, `HUMAN_COLUMNS`, `AGENT_COL_OF_PHASE` and
`agentColOf`. In `the_page_has_a_word_for_every_value_the_server_sends`, replace the checks of the
two dropped tables with Rust tests that every value lands where it lands today, and move the tests
that pin `humanCol`. The board must look the same.
```

**3. adjutant cannot tell an investigation or a design from an implementation**

```markdown
## What happens

Every task is handled as an implementation. Only `--done-when report-only` hints that a task is an
investigation, and it says where the work stops, not what it is. A design task looks the same as
any other.

## Proposal

Add an optional `work` field to the task (`implement`, `investigate`, `design`), set by `adj task
add --work` and changed by `adj task update --work`, refused when unknown. An absent one reads as
`investigate` for `report-only` and `implement` otherwise. Add `work` to the card's facts and show
it as a pill. The hub procedure passes `--work investigate` and `--work design`. Update the README
and both languages' help.
```

**4. A team cannot lay out the board's columns its own way**

```markdown
## What happens

Every repository gets the same columns. A team whose work has an investigation step or waits on
other people has nowhere to put those cards.

## Proposal

Add the `board` setting from `docs/board-columns.md` (repository > `defaults` > top level, read on
every poll): `preset`, `agentColumns`, `humanColumns` and `workTypes`, with the `default` and
`investigation` presets. Check a layout against the fact vocabulary and fall back to the preset as a
whole with a warning in `adj config`, `adjutant_config` and on the board for each problem listed in
the doc. 「すべて」 asks each board for the `default` layout. Document the setting in
`config.example.json` and both READMEs.
```

**5. A card cannot be moved into a column by hand** (after #555)

```markdown
## What happens

A state adjutant cannot know, like waiting on a product manager, has a column only if a person can
put a card there. Parking (#555) records the reason on the task, but no column shows it.

## Proposal

Add `parked` to the card's facts (absent once the task is `done` or `cancelled`), `parkReasons` to
the `board` setting, and the `waiting` preset with its parked columns early. Dropping a card on a
column whose condition names `parked` parks the task with that reason, and dropping a parked card
on any other column unparks it; other drops stay refused. `yourTurn` and the counts leave parked
tasks out. Document `parkReasons` and the `waiting` preset in `config.example.json` and both
READMEs.
```
