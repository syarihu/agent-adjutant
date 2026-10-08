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
phase (`src/ui/board.js`). Only its coverage of the phases is tested (every phase has an entry in `AGENT_COL_OF_PHASE`), not
its precedence: [#466](https://github.com/syarihu/agent-adjutant/issues/466)
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
- Through `humanColOf` or `columnOf`: the shelve and requeue actions (`ALLOWED` in `move`; the
  board has no drag and drop), the page's waiting counts
  (`waitingIn`, `renderCounts`), the panel's 「…を待っています」 pill, the card's waiting mark, the
  stuck strip, and the agent board's sort (cards waiting on the person go last).
- Gates with no card on the board are placed by `g.humanCol`.
- The server's sidebar counts (`board_counts`) read an open gate or `waitsOnPerson`.
- `FINISHED_PHASES` in the sessions view and the stuck badge (`stuckOf`) read the phase.

The session ledger's `waiting` changes no column; it marks the card and fills 要対応.

No placement rule reads the task's `kind` or `doneWhen` (`doneWhen` shows only as a pill on the
card, 調査のみ and the like). A report-only task's worker sets phase
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
| `jules` | Jules's state, lower case with `_` as `-` (`IN_PROGRESS` is `in-progress`); an open set, which the layout check does not hold to a list | The Jules session, for a Jules task |
| `work` | `implement`, `investigate`, `design`, and any added in config | The task's new `work` field ([below](#what-kind-of-work-a-task-is)) |
| `doneWhen` | `report-only`, `verify`, `pr`, `review` | `task::DoneWhen` |
| `gate` | `plan`, `diff`, `verify`, `dispatch`, `issue`, `question`, `result`, `relay` | The kind of the task's open (waiting) gate |
| `prTurn` | `draft`, `unrequested`, `other-reviewer`, `checks`, `changes`, `merge`, `ci-failed`, `merged`, `closed` | `task::PrTurn` from the stored `prStatus` |
| `waitsOnPerson` | `yes`, absent | `waits_on_person`, unchanged |
| `session` | `idle`, `running`, `waiting`, `done`, `failed` | The ledger row of the task's worker session (a status this binary does not know reads as absent) |
| `parked` | A park reason (`pdm`, `design`, `review`, `merge-timing`, `other`, and any added in config) | The task's park ([#555](https://github.com/syarihu/agent-adjutant/issues/555)); absent once `status` is `done` or `cancelled` |

A phase this binary does not know reads as absent too, so a record written by a newer binary never
breaks a card. `session` comes from the ledger rows a board's own poll already reads for its
cards. Where a poll does not read them (「すべて」 asks with `sessions=0`, and `board_counts` reads
no ledger), or the ledger could not be read, `session` is **unread**, which is not the same as
absent: a condition on `session` matches neither, and the views below say "not known" rather than
"no session". The layouts that 「すべて」 uses do not name `session` ([below](#columns-as-conditions)).

`step` is the one derived fact: where work in progress stands, whoever does it. It is `plan`,
`implement`, `self-review` or `pr`, computed exactly as steps 2 to 5 of `agentColOf` above, in that
order: for Jules, a PR or `COMPLETED` first, then the Jules state, then the open gate (status `pr`
alone does not say `pr` for Jules); for a worker, `phase`, then the open gate, then `pr` for status
`pr` or a PR; and `plan` when nothing says anything. It is absent when the task is
not in progress (status `backlog`, `queued`, `done` or `cancelled`), so a column that names `step`
never catches a finished task. A bare session (a worker with no card) has no `status` and gets
`step` from its phase, or `implement` with no phase, as `bareCol` does today.

The task panel's stepper reads `status` and `step`, not a column, so it stays the same whatever the
layout: nothing lit before the task starts, every step lit once it is `done` or `cancelled`, and
`step` in between (the step `self-review` lights the stepper's `selfreview`).

A parent's hub card's gates live in the parent hub's directory, never among the repository
board's gates. Today `ownerHub.humanCol` reads the hub's open gate, while `agentColOf` looks for the
card's id among the board's gates, where it finds nothing or another task's gate with the same id
(ids are unique only per hub). The facts under `ownerHub` take the hub's open gate for both halves;
that changes the agent column of a hub card only where today's lookup was wrong.

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

Without 調査 that is the `default` preset's agent side: `before` (`status`: `backlog` or `queued`),
`plan` (the fallback), `implement`, `selfreview` and `pr` (`step`: `implement`, `self-review`, `pr`),
`done` (`status`: `done` or `cancelled`). Today's human side, written the same way, is `dispatch` (`gate`: `dispatch` or `issue`), `plan`
(`gate`: `plan`), `diff` (`gate`: `diff`), `verify` (`gate`: `verify` or `result`), `prreview`
(`gate`: `relay`, or `gate` absent and `waitsOnPerson`), `question` (`gate`: `question`). The
`gate` absent in `prreview` keeps today's rule that an open gate outranks the PR: a card with a
`question` gate and a PR waiting on the person stays in 質問.

**Where it runs.** The server evaluates, in `board::view`. What it sends comes in pairs:

- Each card carries `facts`, `agentColumn`, `humanColumn` and `yourTurn` (below) computed
  **without** its open gate.
- Each open gate carries the same four computed from its card's facts **with** the gate.
- The page reads the open gate's set, else the card's, as `humanColOf` does now. When the page
  drops a gate it just answered, everything that hangs on the gate (both columns, the stepper, the
  waiting mark, the counts) falls at once to what the next poll will say, as today.
- A gate with no card on the board is evaluated with only its own `gate` fact.
- A parent's hub card carries its set under `ownerHub`, evaluated with the hub's open gate (above).
  The page reads only `ownerHub`'s set for a hub card, never the board's gates; it never drops a
  hub's gate optimistically, so no pair is needed there.
- Every worker row, and every worker entry of `state.sessions` (a different struct, read by the
  sessions view), carries `agentColumn`, computed from its own `phase`, `step` and `session`. The
  page still decides which rows are bare and places them by it, and the sessions view labels a
  row's phase with that column's label instead of `AGENT_COL_OF_PHASE`.
- The new keys are written by the card, so like `humanCol` today they are scrubbed from a record's
  `extra` before the card adds them, and a stale key on disk never comes out twice.

The gate's `humanCol` and `ownerHub.humanCol` are replaced by `humanColumn`; `waitsOnPerson` stays,
as a fact. The tests that pin the old names move with them:
`the_page_places_a_pr_by_its_turn_and_says_when_the_poll_has_stopped` in
`src/transport/board_http/tests.rs` (the page string `t.waitsOnPerson ? 'prreview'`), `humanCol` and
`ownerHub.humanCol` in `src/board/tests.rs` and `tests/board_state.rs`; the exact-JSON fixtures in
`tests/board_state.rs` grow the new fields.

**What a card shows does not follow the column.** Today the page picks a human card's buttons, its
PR-turn pill and its question box by the column id (`humanCard` in `src/ui/cards.js`), the panel's
quick actions by the gate's `humanCol`, and the agent board's queue sections, 次を流す, done
folding and narrow width by `before` and `done`; the human board's PR確認 and poll warning hang on
`prreview`. Under a layout those ids mean nothing. So:

- A card's content and its gate's actions follow the gate's `kind` and the PR's `prTurn`, never
  the column. A `question` gate in a column called `pdm`, or in `waiting`'s 置いている, keeps its
  answer box.
- The column-level widgets attach to a `role` a column can carry: `queue` (the 待ち / Backlog
  sections and 次を流す) and `done` (folding old cards) on the agent side, `prreview` (PR確認 and
  the poll warning) on the human side. At most one column per side carries each role; the
  `default` preset gives them to `before`, `done` and `prreview`. Narrow width goes with `queue`
  and `done`, as now.
- `label`, `icon` and `hint` come from the config, so the page escapes them like any other text
  (today they are constants put straight into the HTML).

Shipping the layout to the page and evaluating there was the other option. It would put the
condition language in JavaScript next to the Rust one, which is what #466 removed, and the page has
no tests that run. Computing the columns twice per card is cheaper than a second evaluator.

The server also sends the layout itself (`columns: { agent: [...], human: [...] }`, with ids,
labels, icons, hints and roles; not `layout`, which the page already uses for its tabs and split), and the page renders the board from it. `AGENT_COLUMNS`, `HUMAN_COLUMNS` and
`AGENT_COL_OF_PHASE` leave the page.

**Ids are unique per side.** The default layout keeps today's ids, including `plan` on both sides,
so the page code that still names them (for instance to keep scroll positions across a render)
goes on working. Wherever a column is named outside its own board (a
label in the task panel, a stored filter), the page names it with its side (`agent:plan`,
`human:plan`), so the mix-up behind [#488](https://github.com/syarihu/agent-adjutant/issues/488)
(since closed) cannot come back with a layout that reuses an id.

**「すべて」** merges several boards in the page, and their layouts can differ. It asks each board
for its state with the `default` layout (`/api/state?columns=default`), and draws `default`'s
columns; the state `mergeStates` builds carries those `columns`. A board's own page uses its own
layout. The cross-repository view that replaces 「すべて」
([#553](https://github.com/syarihu/agent-adjutant/issues/553)) reads facts instead, as below.

## Presets and the config

The key is `board`, a machine setting like `stuckAfterMinutes` (a `SETTING_KEYS` entry, with an
"an object" shape in `accepted_shape`): read again on every poll, so a change shows without a
restart. Like most settings it is taken **whole** from the most specific level that has it (the
repository's entry, then `defaults`, then the top level), not merged key by key as `terminal` is. In
the example below, `acme/web` names its own `preset` because nothing comes over from `defaults`.
`workTypes` and `parkReasons` are separate settings of their own, not keys of `board`, so a
repository that lays out its own board does not lose the types and reasons from `defaults`.
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
- The settings `parkReasons` and `workTypes` add values to `parked` and `work` (`[{ "id": "legal",
  "label": "法務の確認待ち" }]`), resolved the same way as `board`. Built-in values cannot be
  removed or relabelled. `adj task park --reason` and `--work` accept them through one check each in
  `task`, which the layout check calls too.
- An empty config is the `default` preset, which is today's board exactly: the same twelve columns,
  ids, labels, order and placement.

Presets shipped:

| Preset | Agent side | Human side |
|---|---|---|
| `default` | Today's six | Today's six |
| `investigation` | Adds 調査 (`work: investigate`, `step: "*"`) and 設計 (`work: design`, `step: "*"`) after 着手前, so those tasks stay in one column whatever their phase while they are in progress | Today's six |
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
- a `gate` value no human column matches: the human side has no fallback, so such a gate would be
  open, counted as the person's turn, and nowhere on the board;
- a `when` on the fallback, or a column other than the fallback with none;
- a human side on which a card with no gate and `waitsOnPerson` reaches no column: it would be
  the person's turn and nowhere on the board;
- a column nothing can reach: one whose `when` is an empty list (`[]`), one after a column whose
  `when` is empty (`{}`), and one whose
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
the task was handed over, and it shapes the request text and whether creating the task reads the
issue. The two stay separate: one says what
the person asked the hub to do, the other what the work is.

- It is stored as a plain string, not a closed enum as `kind` is, so a configured type never stops
  a record from loading. A stored type the config no longer lists reads as that word: it shows as
  it is and matches only a `"*"` condition. The park reason is stored the same way.
- An absent `work` reads as `investigate` when `kind` is `investigate` or `doneWhen` is
  `report-only`, else `implement`. Old
  records need no change, and a binary that does not know `work` keeps it in `extra` (rule 10 in
  [architecture.md](architecture.md)).
- `adj task add --work <type>` sets it, and `adj task update --work <type>` changes it: a task that
  starts as an investigation can become an implementation. An unknown type is refused before
  anything is written. The check is one function in `task`, given the configured `workTypes`, and
  the layout check in `board::view` calls the same one, so the two cannot disagree.
- Every route that creates a task can set it. The hub passes `--work investigate` for an
  investigation-only request and `--work design` for a design task to `adj task add`. The board's
  new-task form and its link dialog gain a 作業 picker (実装 / 調査 / 設計 and the configured
  types), defaulting to 調査 when 種類 is 調査だけ. A task that already exists is changed with `adj
  task update --work`. The worker never sets it.
- `doneWhen` keeps its meaning (where the work stops) and its 調査のみ pill. `work` shows as a
  pill of its own only when it is not `implement`. `work` does not change any procedure, only
  facts.

## Columns a person moves cards into

Some states adjutant cannot know: waiting on a product manager's confirmation, on a designer, on
the right moment to merge. [#555](https://github.com/syarihu/agent-adjutant/issues/555) parks a task
with a reason, on the task record, from the board or the CLI. **A manual column is a column whose
condition is a park reason.** There is no second mechanism: parking is the one fact a person sets by
hand, and a layout decides where parked cards show.

- **Into it**: the board has no drag and drop, and this design does not add it. A person parks a
  task with #555's 置く (a reason picker on the task summary and the decision dock) or `adj task
  park`; the card then goes to the first column whose condition its park reason matches. The
  picker lists the reasons the board's layout has a column for first.
- **Out of it**: 置くのをやめる or `adj task unpark`. The card goes back to where its other facts
  put it.
- **What stops a fact from pulling it out**: nothing moves a parked card but a person, as long as
  the parked column comes before the columns its other facts match. The `waiting` preset puts it
  first on the human side and right after 着手前 on the agent side. A layout can put it later; the
  warnings catch only the plain unreachable cases above, so the README says to keep parked columns
  early.
- **What ends it anyway**: the `parked` fact is absent once the task is `done` or `cancelled`, so
  a merged PR still takes the card to 完了, even when the binary that wrote `done` does not know
  about parking and left the park on the record. Clearing it when the status is written is only
  tidying.

## Counts, 新着 and the other views

The board's counts and the cross-repository 「いまの仕事」 view
([#549](https://github.com/syarihu/agent-adjutant/issues/549),
[#553](https://github.com/syarihu/agent-adjutant/issues/553)) read facts, not columns. Columns
differ from one repository to the next, and a layout must not change what "it is your turn" means.

- **Whose turn it is** stays independent of the layout: an open gate, or `waitsOnPerson`, and
  not `parked`. The server sends it as `yourTurn`, in the pairs above (the card's without its gate,
  the gate's with it), and computes it the same way for `board_counts`. Everything listed earlier that reads `humanColOf` to ask "is this the person's"
  rather than "where does it go" (the page's counts, the 「…を待っています」 pill, the card's waiting
  mark, the stuck strip, the agent board's sort) reads `yourTurn` instead. A parked card in the
  `waiting` preset's 置いている column is shown on the human half but is not the person's turn and is
  not counted. The sidebar counts, the badges and 新着
  ([#554](https://github.com/syarihu/agent-adjutant/issues/554)) use it.
- #553's progress segments (merged / PR / working / not started) are a fixed derivation over
  `status`, `prTurn` and `step`, written once on the server next to the facts. Its state boxes (新着
  / 後で見る / 実行中 / そのほか) add `yourTurn`, `parked` and `session` from the server and the
  browser's seen marks from #554, which only the page has, so the page sorts cards into them. Both
  can show a card's column label from its own repository as text.
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
  implementation step puts `agentColOf`'s rules on the server as the `step` fact, with a table test
  of every branch listed above, while the page still places cards itself; the second makes the page
  render from what the server sends. The board must look the same before and after each, and the
  tests say so.
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

**1. The agent board's column is decided only in the page, and its precedence is untested**

```markdown
## What happens

Where a card sits on the agent board is computed in the page (`agentColOf` in `src/ui/core.js`),
with branches for Jules, the worker's phase and the open gate, and no test checks their order.
Every other view that wants the same answer (the task panel's stepper, the sessions view) has to
read it from the page or compute it again, and the server cannot give it to a view of its own.

## Proposal

Add the facts from `docs/board-columns.md` to `board::view`: one struct with `status`, `step`,
`phase`, `executor`, `jules`, `doneWhen`, `gate`, `prTurn`, `waitsOnPerson` and `session`, sent as
`facts` on each card without its open gate and on each open gate with it, under `ownerHub` with the
hub's open gate, and on each worker row and worker session entry (`phase`, `step`, `session`).
`step` reproduces `agentColOf` branch by branch; port its precedence into a table test, bare
sessions and a parent's hub card included. `session` is unread where the poll reads no ledger. Scrub
the new key from a record's `extra` as `humanCol` is. The task panel's stepper reads `status` and
`step` (nothing lit before the task starts, all lit once done or cancelled). No change on the board.
```

**2. The board's columns are hard-coded in the page**

```markdown
## What happens

The two halves of the board are fixed lists in the page (`AGENT_COLUMNS`, `HUMAN_COLUMNS`,
`AGENT_COL_OF_PHASE`), and what a card shows depends on the id of the column it is in. A different
layout means changing the page, and the server cannot check that every value it sends has a
column.

## Proposal

Add the column evaluator and the `default` preset from `docs/board-columns.md` to `board::view`.
Send `agentColumn`, `humanColumn` and `yourTurn` alongside each `facts` from the first step, and
`columns` (ids, labels, icons, hints, roles) in the state, carried through `mergeStates`; the
gate's `humanCol` and `ownerHub.humanCol` give way to `humanColumn`. The page renders both halves
from `columns`, places cards by those fields, reads `yourTurn` wherever it asks whether a card is
the person's, picks a card's content and actions by its gate's kind and PR turn rather than the
column id, hangs the queue sections, done folding and PR確認 on column roles, escapes column text,
labels a session's phase by its column, and names columns with their side outside their own board.
Drop `AGENT_COLUMNS`, `HUMAN_COLUMNS`, `AGENT_COL_OF_PHASE` and `agentColOf`. Replace the page
test's checks of the dropped tables with Rust tests that every value lands where it lands today,
and move the tests that pin `humanCol`. The board must look the same.
```

**3. adjutant cannot tell an investigation or a design from an implementation**

```markdown
## What happens

Every task is handled as an implementation. Only `--done-when report-only` or the hand-over kind
`investigate` hints that a task is an investigation, and neither says what the work is. A design
task looks the same as any other.

## Proposal

Add an optional `work` field to the task (`implement`, `investigate`, `design`), stored as a plain
string, set by `adj task add --work` and changed by `adj task update --work`, refused when unknown
by one check in `task`. An absent one reads as `investigate` when `kind` is `investigate` or
`doneWhen` is `report-only`, and `implement` otherwise. Add `work` to the facts and show it as a
pill when it is not `implement`, next to the 調査のみ pill. Give the board's new-task form and link
dialog a 作業 picker, and have the hub procedure pass `--work investigate` and `--work design`.
Update both READMEs and the help.
```

**4. A team cannot lay out the board's columns its own way**

```markdown
## What happens

Every repository gets the same columns. A team whose work has an investigation step or waits on
other people has nowhere to put those cards.

## Proposal

Add the `board` setting from `docs/board-columns.md` (taken whole from the repository, `defaults`
or the top level, read on every poll) with `preset`, `agentColumns` and `humanColumns`, and the
`workTypes` setting, which `--work` then accepts through the same check. Ship the `default` and
`investigation` presets. Check a layout against the fact vocabulary and fall back to the preset as
a whole, with a warning in `adj config`, `adjutant_config` and on the board, for each problem the
doc lists. 「すべて」 asks each board for the `default` columns (`?columns=default`). Document both
settings in `config.example.json` and both READMEs.
```

**5. A parked task has no column of its own** (after #555)

```markdown
## What happens

A state adjutant cannot know, like waiting on a product manager, can only be told by a person.
Parking (#555) records the reason on the task, but no column shows it, and a parked task stays in
the column its other facts give it.

## Proposal

Add `parked` to the facts (absent once the task is `done` or `cancelled`), the `parkReasons`
setting (accepted by `adj task park --reason` through one check in `task`), and the `waiting`
preset with its parked columns early. Parking and unparking stay #555's actions (置く with a
reason, 置くのをやめる, `adj task park` / `unpark`); the reason picker lists first the reasons the
layout has a column for. `yourTurn` and the counts leave parked tasks out. Document `parkReasons`
and the `waiting` preset in `config.example.json` and both READMEs.
```
