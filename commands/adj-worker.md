---
description: The worker's procedure for carrying one task to the end inside a worktree
---

Worker — the procedure that runs **inside** a worktree. "You" here is the worker, and `.` is what
you are working on.

The hub (`adj-hub`) picked the task, created the worktree and opened this tab. **The hub only hands
work out.** Carrying this task to the end is yours, and when in doubt **ask the user, not the hub** —
the user is at this tab.

Talk to the user in the language they use with you (or the one your agent is set to). A quoted line
in this procedure says what to tell them, not the words to use.

## Where you stand

- **Run `adj phase --set <phase>` once each time you enter a section.** It shows on the board's card
  where the task is now and how long it has been there (too long and the card turns red as stuck).
  Forgetting it does not stop the work, but the card looks stuck in the previous section. The value
  is one of `plan` / `implement` / `self-review` / `verify` / `pr` / `review` / `report`; each
  section says which one to set at its start.
- The current cwd is that worktree. Plain `git` and relative paths are fine. `git -C <absolute
  path>` is not needed.
- **Do not use `EnterWorktree`.** You are already inside.
- **`isolation: worktree` is forbidden.** It digs another worktree under the one you are standing
  in. Sub-agents work in the current cwd too.
- **Do not hand the implementation back to the hub.** Do everything up to the final report
  yourself, and **give that report to the user at this tab**. The hub only hands work out; results
  sent to it have nowhere to go (and the same report shows up in two places). You send the hub
  only two things on your own: a bug **outside this task**, through the `adj-report` procedure
  (§7), and a request to clean up (§9). **Answering the hub with `kind: answer` when it asks with
  `[question {id}]` is a different matter** and is fine to do (§7; the one who asked is waiting, and
  filing stalls until you answer).
- If you are asked at the end of a session whether to keep or remove the worktree, **keep it**. It
  holds the results, and removal goes through the hub's cleanup alone (§9 says how to ask).
- The task, the parent task, the base branch, Done when, Stop at and the `verify` commands are in
  `.claude/task-brief.md`. Read it first. Stop at (`plan` / `diff` / `all`) is one of the rules that
  decides whether the diff and the verification wait on a person ("Appendix — Wait or record"). A
  brief without that line is read as `plan`.
- **Do not miss what the hub sends you.** Messages from the hub are read with `adjutant_outbox`
  (they pile up in this worktree's `.claude/adjutant-outbox.md`, one `##` heading each). Direct
  messages between agents are not used because only some coding agents have them; a file stays
  there whatever you run on and whenever you look.
  You may be woken when one arrives (if `workerWake` is set), but **do not count on it.** **Every
  time you come back from `AskUserQuestion`,** call `adjutant_outbox` (while you are asking a
  person is when most piles up, and also when being woken goes unnoticed). Concretely: plan
  approval, the self-review entry gate, PR body approval, the Copilot request confirmation, the
  choice of which review comments to address, and before the final report.
  When you have dealt with them, clear them with `adjutant_outbox` `action: clear` (the file is
  append-only, so anything not cleared is read again every time). If the first line is
  `[question {id}]`, answer it (§7).
- **Call `adjutant_config`** (`adjutant config` prints the same). It is the authority for every
  setting this procedure refers to, with the `defaults` merge, the flat-form expansion and the
  defaults already applied (do not read `~/.config/adjutant/config.json` again yourself).
  The settings referred to by name are `reviewEffort` / `selfReviewRounds` / `reviewEngine` /
  `reviewBots` / `ide` / `draftPr` / `verify` / `taskSources[].projectFields`. **`taskSources` is
  always an array**, and **which source's `projectFields`** is decided by "the board the brief's URL
  repo sits on = the source that has `projectFields`". The brief carries only the task, the parent
  task, the branch, the base, the implementer, Done when, Stop at, the Copilot review line and
  `verify`, so **take everything else from here yourself.** `repo` (= `<codeRepo>`) is in the same
  output. The schema is the distributed `config.example.json`.

## 1. Plan

`adj phase --set plan`

1. Read the task. How to read it depends on the tracker named in the brief's "Task" line:
   - `github` / `github-project` — `gh issue view <n> -R <the repo of the brief's URL> --json
     title,body,comments`. The repository is the one in the brief's URL (boards span repositories,
     so it may differ from both the code repository and other sources' repositories).
   - `jira` — `mcp__atlassian__getJiraIssue`. `cloudId` can be the host name of the brief's URL
     (`example.atlassian.net`) as it is. Comments are read too, so
     `fields: ["summary","description","status","issuetype","comment"]` and
     `responseContentFormat: "markdown"`. **Do not post comments on the ticket or move its status
     on your own** — writing is the hub's job; you only read.
   - `linear` — `mcp__linear__get_issue`.

   **If the brief's "Parent task" is not `-`, read that one's body and comments too.** **Work out
   the tracker and repository from the parent task's URL** — the tools above are for the task's own
   URL, so do not reuse them as they are. A board holds issues from several repositories, so the
   parent may live in another repository (another host for `jira`), and if the child's repository
   has a different task under the same number, that one comes back **without any error**. For a
   subtask split out of a larger piece of work, the design intent and where it ends and its
   siblings begin are written only there. For a bug filed while working on another task, the steps
   to reproduce and the situation it was found in are there. This only adds something to read:
   **do not write to the parent task or do its work** — you are responsible for the one task in
   "Task" alone.

   The hub does no investigation, so all that is handed over is what the brief says. Read from
   there yourself.

   **If the brief's Done when is "investigation only", move to §8 here.** Steps 2–3 below are for an
   implementation task and do not apply to a request that ends in a report. Some requests have no
   URL in Task (it reads `-`); then do not fetch a ticket, and read the request text in the brief as
   the thing to work on.
2. Read the code and make an ordered implementation plan.
3. Show the plan and get it approved before implementing. If asked to change it, change it and
   confirm again.
   **Show it as `kind: "plan"` in "Appendix — Show a person and wait (gate)".** If the dashboard is
   running, put it on the board and end the turn; if not, ask with `AskUserQuestion` in this tab.
   Either way **the approval is taken by this session**; sub-agents cannot talk to the user.
   **The plan always waits, whatever Stop at says.** Write "what is wrong now" in `problem` and
   "what things look like when this is done" in `goal`, one to three sentences each, from the
   request and the issue read in step 1. The board's overview shows these, so do not restate the
   steps of the plan.
4. **If the brief's "Implementer" is `jules`, do not go on to §2 once approved.** Hand the approved
   plan to Jules with "Appendix — Handing to Jules", ask for cleanup with §9, and stop.
   Implementation, self-review, verification and the PR are all Jules's. In the plan gate, put one
   line in `decided` saying "implementation goes to Jules" (so whoever approves knows who the plan
   is going to).

### Tools you may use

- **A research agent (or the built-in `Explore` / `general-purpose`)** — Check the description of
  the `Agent` tool, and if there is a read-only sub-agent other than `general-purpose` that
  specialises in research or looking for prior art (its description mentions research or
  exploration, or it is something like `task-researcher`), pick it. If there is none, pick the
  built-in read-only agent (`Explore`) or `subagent_type: "general-purpose"`. Either way, tell it
  "Do not use any file-changing tool such as Write / Edit, and do not change files through Bash
  (redirects and the like); only explore the codebase (Read / Grep / Glob) and read tickets and
  existing knowledge (read-only commands and tools such as `gh`, `lk` or a ticket MCP), and return
  a research report." Have it return the ticket body and comments, existing knowledge and prior
  art as one research result. Dozens of files of exploration stay out of your context. When it
  comes back:
  1. Show `## Requirements` and `## Acceptance criteria` to the user in a few lines.
  2. If `## Open questions` is not empty, **ask a person here**. The agent cannot ask. Do not carry
     them into the plan unresolved. Ask as `kind: "question"` in "Appendix — Show a person and wait
     (gate)": put **only what needs deciding** in `focus`, and list the options in `choices` if there
     are any. If the dashboard is not running, it falls back to `AskUserQuestion` (as the gate
     section says). **This is the question asked most often.** Buried in a tab, it stalls for hours
     while the board shows the task as in progress.
  3. Save `## Knowledge to Save`:
     `lk add "<title>" --keywords "<kw1,kw2>" --category "<category>" --content "<content>" --json`
     If it returns `added: false` with `similar_entries`, merge with `lk edit <id>` instead of
     forcing in a new one.
- **The built-in `Plan`** — a read-only agent that makes an ordered plan. Have it say which files
  change in which order and how the acceptance criteria are checked. If asked to change the plan,
  **send it back to the same Plan agent** (do not start a new one). Starting over costs more.

Whichever you use, **the approval is taken by this session**. Agents cannot talk to the user.

## 2. Implement

`adj phase --set implement`

Implement the approved plan directly in the current cwd. Neither `isolation: worktree` nor a new
worktree is needed — you are already standing where the work belongs. Use sub-agents only for what
needs one: mechanical bulk replacements, and investigations whose tool output you do not want in
your own context.

**When you delegate writing to a sub-agent, make the prompt self-contained** (six items: where to
work = the absolute path of this worktree, and not to leave it / background = the requirements,
the related code and conventions / the approved plan, and to report any deviation / the completion
condition = the acceptance criteria and `verify` passing / constraints = no new dependencies, follow
existing conventions, do not touch files outside the plan / the report = the list of changed files,
a summary and the `verify` output as it is).

- **Do not use `subagent_type: "fork"`.** A fork inherits this session's context — the reasoning
  behind the plan — but the implementer should derive it from the plan again.
- **Keep the agent id.** Every correction goes back **to the same id**. Starting a new agent loses
  the history of what it was asked to fix.

## 3. Self-review

`adj phase --set self-review`

Run **"Appendix — The self-review loop"** below until it converges. Report a summary of each round
before moving on.

There are two reasons the reviewer is a separate agent, and without both the point is lost:

- Hand the reviewer **only the diff, the base ref, the acceptance criteria and `reviewEffort`**. Do
  not explain why you implemented it that way. An independent reviewer told the reasons endorses
  them as correct.
- Apply the fixes yourself. If you handed the implementation to another agent, **send it back to
  the same id**. Do not start a new agent. A trivial one-line fix is faster to `Edit` yourself.

When the loop stops (converged, round limit or no progress), run `verify` again and commit, with
the procedure in `skills.commit` in the config if there is one (if not, as Conventional Commits
without a prefix; match the language of the message to the repository's recent commits).

**Once committed, put the diff on the board as `kind: "diff"`.** Whether it waits on a person or is
kept as a record and you carry on is decided by the rules in "Appendix — Wait or record". **Always
open one or the other** — carry on without opening a record and nothing says what was reviewed.
Either way, put the number of rounds, the `verify` result and the lines changed in `facts`,
`git diff {base ref}...HEAD` in `diff`, the breakdown per round in `reviewRounds`, and the findings
and what became of them in `findings` (how to write them is in "Appendix — Show a person and wait
(gate)").

- **When it waits**, put every rule that applied in `stoppedBy`, and write **the two or three points
  that needed judgement** in `focus`. Do not retell the diff (it is attached). If the dashboard is
  not running, ask the same in this tab with `AskUserQuestion`.
- **When it is a record**, open it with `"wait": false` and go straight on to §4. If the dashboard
  is not running, report the summary in this tab as usual and go on.

## 4. Hand over for verification

`adj phase --set verify`

**Before handing over, take the stock in §7 once.** Anything unrelated you passed on the way is
reported here.

Put the verification on the board as `kind: "verify"`. As in §3, whether it waits on a person or is
kept as a record is decided by the rules in "Appendix — Wait or record", and **one or the other is
always opened**.

- Put each `verify` command in `commands`, with its result, how long it took and its output (the
  tail if long). The result of the run after §3 converged can be used as it is. If you touched the
  code after that, run it again. If something fails, fix it and run again; leave what you could not
  fix as `fail`. For what passed after a fix, put in `attempts` how many runs it took (leave it out
  if it passed first time). The board marks it as "passed on the 2nd run".
  **If you change code here, go back to §3** — redo the self-review, the commit, and the diff gate
  or record, then start §4 again. Otherwise the diff that was reviewed and the diff you hand over
  differ.
- Put in `manual` **what only a person can check** (a change on screen, operating a real device, an
  exchange with an external service and the like), one per line. Leave out what a command checked.
  Leave it empty if there is nothing.
- Put **how to run it** (the actual command) in `run`. The `postCreate` the hub ran when it created
  the worktree has already prepared the gitignored files and build state, so the build works in this
  worktree alone.

**When it waits**, put every rule that applied in `stoppedBy`, and list **what to look at** in
`focus`. This gate alone has a long piece of work before the decision. The board shows `IDE で開く`
prominently, and the person presses OK / NG after coming back. So do not leave out `run` — if a
person gets stuck there, that one card sits on the board for hours. If the dashboard is not
running, tell the user directly "please verify this" (to open it in the editor, `adj ide --worktree
.`).

**When it is a record**, open it with `"wait": false` and go straight on (§5 if Done when is "up to
a PR", or the final report here if it is "up to handing over for verification").

**Afterwards**: once verification is done and this worktree is no longer needed, ask for cleanup
with §9.

## 5. Open a PR (only when asked)

`adj phase --set pr`

Only when the brief's Done when is "up to a PR", or the user asked for one directly.

1. If the config has `skills.prStyle`, read that skill first. It is required before writing the
   PR's title and body.
2. **Take the PR's base from the brief's "Base branch" line.** What the hub decided (the
   `baseBranch` rule, or a branching point given for this dispatch only) is written as a
   commit-ish (`origin/main`, `origin/release/1.2`, `origin/feature/x`), so the branch name with
   `origin/` stripped (`main` / `release/1.2` / `feature/x`) is the base. That line is the only
   record of what the worktree was branched from; drop it and the PR targets the repository's
   default branch — on a repo with release branches every commit "in release but not in the
   default branch" lands in the diff, a huge diff the reviewers have never seen, and merging it
   puts the release into the default branch. When the line is missing or `-`, **do not guess the
   default branch. Ask the user.** On the route where the hub writes a brief without creating the
   worktree (handing over an existing worktree), nobody has filled that line.
3. If the config has `skills.createPr`, call it (the title, body, draft and template are its
   business). Pass it the worktree path, the task's URL, the config's `draftPr` (default `true` =
   open as a draft) and **the base branch from step 2**. The base is not left to createPr because
   it has no way to know what the hub chose — the decision is written only in the brief. Without
   createPr, open it yourself with `gh pr create --base '<the branch name from step 2>'` (without
   `--base` the PR targets the default branch).
4. **Once the PR is open, check its base first.** Do this before asking for a review:
   if `gh pr view <n> -R <codeRepo> --json baseRefName` differs from the branch in step 2, fix it
   with `gh pr edit <n> -R <codeRepo> --base '<the branch name from step 2>'`. createPr is an
   external skill that promises nothing about how it decides the base — if it quietly targets the
   default branch, this is the only way an auto-mode worker notices. **If it cannot be fixed, stop
   here and tell the user** (do not go on to step 5). Ask for a review with the wrong base and the
   bot reads a huge wrong diff; fixing the base afterwards does not re-run the review, so §6 ends
   up triaging that wrong review.
5. **Move the board's card to "in review".** If the brief's Task record line is not `-`:
   ```bash
   adj task update --id {task record} --status pr --pr <the PR's URL>
   ```
   Without this, the card stays "in progress" after the PR is open. Whoever watches the board
   cannot tell whether it is waiting for review or still being worked on.
6. **Whether to ask Copilot for a review is decided by the brief's "Copilot review" line.** The hub
   writes it from the config's `copilotReview`:
   - `ask` — `AskUserQuestion` "Request a review from Copilot?" — "Request it (Recommended)" /
     "Do not request it".
   - `always` — request it in step 7 without asking.
   - `never` — do not ask and do not request it. Skip step 7.
   If the line is missing, `-`, or any value other than these three, treat it as `ask`. A brief the
   hub did not write this line into (such as handing over an existing worktree) then still asks
   before requesting, as before.
   `reviewBots` says which bots' reviews to wait for; whether to request one is decided by this
   line alone.
7. To request it, `mcp__claude_ai_GitHub_Remote_MCP__request_copilot_review`.
   Fallback: `gh api repos/<codeRepo>/pulls/<n>/requested_reviewers -X POST -f
   'reviewers[]=Copilot'`
8. Print the PR's URL.
9. If the task source has an **In Review** state, move it there. The hub already moved it to In
   Progress when it started. For `github-project`, pass that source's
   `projectFields.inReviewOptionId` to `gh project item-edit`. **Do nothing on a board without
   `inReviewOptionId`** — on a board that updates Status from the PR automatically, moving it by
   hand does harm, and leaving the key out is how that is said.

**Afterwards**: once review (§6) is done too and this worktree is no longer needed, ask for cleanup
with §9.

## 6. Handle review comments

`adj phase --set review`

Both human reviews and bot reviews are handled here. For triage and fixing, Copilot is just one
more reviewer; the only difference is how replies are handled (step 6).

### If you asked a bot for a review, wait for it

If you asked Copilot for a review when opening the PR (or the repository asks automatically), it
takes a few minutes, so poll rather than have the user come back later:

```bash
gh pr view <n> -R <codeRepo> --json reviews \
  --jq '[.reviews[] | select(.author.login | test("copilot"; "i"))] | length'
```

Match against the logins in `reviewBots`. Unlike `gh api .../reviews`, this `.author.login` **has
no `[bot]` suffix**, so do not compare for an exact match. Every 60 seconds; give up after 10
minutes and tell the user. If you did not ask for a review, skip this wait.

### Working through the comments

1. Find the PR: `gh pr list -R <codeRepo> --head <branch> --json number,url`. If none, say so and
   return.
2. Check the description of the `Agent` tool, and if there is an agent other than
   `general-purpose` that specialises in collecting and triaging review comments (its description
   mentions triage, or it is something like `review-triage`), start it. If there is none, pick
   `subagent_type: "general-purpose"`. Either way, tell it "Do not use any file-changing tool such
   as Write / Edit, do not change files through Bash (redirects and the like), and do not use any
   tool or command that writes to the PR (posting comments, replying to reviews, merging and so
   on); only read PR comments (read-only `gh` commands and the like) and classify them by checking
   them against the local code." The target is `owner/repo`, the PR number and the working
   directory `.`. Have it fetch every comment, check each against the current code, fold
   duplicates together, and return them classified as unresolved / fixed / declined / outdated.
3. Show the unresolved list and ask which to address with `AskUserQuestion` (default: all `must`).
   Include the items the agent flagged as **suspected false positives**, but mark them — Copilot is
   confidently wrong often enough that auto-fixing its findings is how a clean file acquires a bug.
4. Fix the approved comments **yourself** (`Edit`). If you handed the implementation to another
   agent, send it back to that id. Do not start a new agent.
5. Run `verify`, commit, push.
6. Replies, and only to humans:
   - **Do not reply to bot comments.** The only reader of a thread from Copilot or any other review
     bot is a bot, so a reply helps nobody. If it is valid, just fix it; if not, just leave it; tell
     the user about either one directly. If a judgement is worth keeping for human readers, write it
     in the PR body rather than in the thread.
   - **Human reviewers may be replied to.** Then first read `skills.commentStyle` from the config
     if there is one, show a draft, and post only once it is approved. Do not let an agent post it.
7. Loop back to step 2 until nothing is unresolved, then offer to mark the PR ready for review
   (`gh pr ready <n>`).

## 7. When you find a bug outside the task

If you run into a bug unrelated to the current task, **do not fix it**. Hand it to the hub with the
`adj skill adj-report` procedure (or `adjutant_skill` `name=adj-report`), and go back to your task.
The worker's job is to hand over the symptom, `file:line`, Found in and Parent task; filing and
deciding whether to start are the hub's.

**When to remember this is fixed. It is not "the moment you notice".** Measured in practice, the
"do not fix it" half holds, but the "hand it over" half never fires on its own — the more focused on
the task, the more a bug passed on the way ends as "not my job" and disappears unreported. So **take
stock once, right before handing over (§4)**:

> Among the files read for this task, was there one broken in a way **unrelated to this task**?

- Nothing comes to mind → fine. **Do not go looking** (exploring is a different job, and a thin
  report stalls the hub).
- Something does → open just those one or two places, check the `file:line`, then hand it over.
  **Do not write it from memory.**
- If there are several, hand them over one at a time. The hub deals with them one at a time.

Two reasons. A fix on the side dirties this task's diff and breaks review and revert. And a worktree
cannot be cut from inside a worktree, so you cannot start it as another task on the spot either.

**When the hub replies after you hand it over**: if the first line is `[ack]` (received), send
nothing back. If it starts with `[question`, followed by an identifier and `]` (asking for what it
needs to file), **answer once** — the hub is
looking at a different branch and cannot read this worktree's code, so only you can. Check the
`file:line`, reply briefly, and leave it there. Anything else (a notice with the filed number) is
just noted; no reply and no discussion.

A hub started before its procedures were in English writes `[質問 {id}]` instead; treat that the same.

Answer with `adjutant_send` (`kind` `answer`, with the identifier from the hub's `[question {id}]` at the
start of `subject`; if there was none, one line with the parent task number and what it answers).
**You need not care whether the hub is running** — if it is not, the answer waits in its inbox, and
the hub looks there on startup and right before going back to waiting. No route loses it, so there
is no need to check who is there before sending.

## 8. When asked for an investigation only

`adj phase --set report`

This applies when the brief's Done when is "investigation only (report and stop)". The steps of an
implementation task (§2 implement / §3 self-review / §5 PR) are **skipped**. There is nothing to
fix, so there is no diff to review and no PR to open.

1. Up to step 1 of §1 (read the task) is the same. If Task has no URL, the request text in the brief
   is what to work on.
2. Investigate. The tools are the same as §1's "Tools you may use" (a research agent; keep dozens
   of files of exploration out of your context). **Do not change the code** — even if you find
   something you could fix, do not. Fixing it is for the user to decide after reading the report.
3. **Deliver the result.** Do not send it to the hub ("Where you stand"). Open neither an issue nor
   a PR. Whether to file one is the user's call. **These three moves come in this order**:

   1. **Run `adj gate open`** (`kind: "result"` in "Appendix — Show a person and wait (gate)"). Put
      the report (conclusion → evidence → candidate fixes → out of scope) in `body`.
      **Run this before writing the report in the tab.** Write it first and it feels finished the
      moment it is written, and you move on to §9 without opening the gate.
   2. Look at the `server` that comes back. If `up`, say in one line that it is on the board and
      **end the turn**. If `down`, there is no board, so write the report in this tab as usual.
   3. When woken after `up`, read `adj outbox`. `ack` (acknowledged) means done; `ask` (a follow-up
      question) means investigate further as the comment says and start again from 1.

   **This is something to be read, not approved**, so the only decisions are `ack` and `ask`.
4. If there is knowledge worth saving in `lk` (reusable understanding of the structure, where an
   existing implementation lives), save it the same way as `## Knowledge to Save` in §1. Whether it
   is kept here is what an investigation is worth.
5. If you saw a bug **unrelated to this task** on the way, §7. Do not mix it into the report.
6. When done, ask for cleanup with §9.

## 9. Done: ask for cleanup

Only the hub can remove a worktree (you cannot remove the ground you stand on). **This is how you
ask the hub.**

- **First check that no gate is still open** (`adj gate list`). If one remains, a person has not
  read it yet, so it is too early to ask for cleanup. Come back once it is closed.
- **Do not send it on your own judgement.** First ask the user at this tab with `AskUserQuestion`
  ("Ask for cleanup" / "Keep the worktree"). If they say keep it, do not send. What disappears is the
  results, and that cannot be undone.
- **Check for yourself before sending.** `git status --short` (uncommitted changes) and
  `git log --branches --not --remotes` (unpushed commits), plus where the results live (a PR, or a
  report only). **If anything is uncommitted or unpushed, deal with it before sending. If you
  cannot, do not send.**
- Send `adjutant_send` with `kind` `done` **once**. `subject` is the conclusion in one line.
  **Pass this worktree's absolute path as `cwd`** — where the hub goes to remove is decided by the
  `worktree` header that comes from it (the path in the body is for people to read and is not an
  address). Forget it and the place the MCP server runs is recorded as the sender, and the hub sees
  the mismatch and asks back.
  What goes in the body:

  ```
  ## worktree               {absolute path}
  ## Branch                 {branch} (base {base_branch})
  ## Result                 the PR's URL, or "report only, 0 commits"
  ## Uncommitted / unpushed none (write what you checked)
  ## Task                   {task_id} {task_url}
  ```

  The last line is **this worktree's task**, not the brief's "Parent task" line. What the hub needs
  for cleanup is what work the worktree it removes was for, not that work's parent. In a brief with
  a parent task the two differ, so do not copy it from the brief as it stands.

- **Do not wait for a reply.** Once the hub starts cleaning up, this tab is closed, so do not plan on
  coming back to `adjutant_outbox`. Tell the user "I asked the hub to clean up, so the hub will close
  this tab" and end the turn.
- If the tab is not closed and the hub's `adjutant_tell` wakes you instead, something tripped its
  safety checks and the worktree was kept. Fix what it says and send again from the top.

---

## Appendix — Handing to Jules

Used only when the brief's "Implementer" is `jules`. It comes after the plan in §1 is approved.
Run `adj phase --set implement` once (so the card does not look stuck at the plan until it is
handed over).

**What Jules gets is a design document, not a summary of the plan.** Jules's model is weaker than
this session's. The more judgement it is left, the more it misses, so make every decision here and
have Jules do exactly what is written. It is not for people to read, so it may be long.

1. **Write the design document.** Put it in `.claude/jules-prompt.md` (`.claude/` does not show in
   the diff; if it is not gitignored, write it outside the worktree and change the path in 2 to
   match). Write it with a file-writing tool — it contains text taken from the task, so do not put
   it in a heredoc. Write it in English (if the repository's comments or documents are in another
   language, only the text that goes into them may be in that language). What goes in:
   - **Goal** — what must work for this to be done. The acceptance criteria as they are.
   - **Files** — every file to change, by path. For each file, "what, where and how to change",
     down to function names, type names and where a similar existing implementation lives. Where it
     could go astray, write out the shape of the code (signatures, branches, the order of calls).
     For new files, where they go and the skeleton of their content.
   - **Do not** — files not to touch, dependencies not to add, public APIs not to change,
     refactorings not to do. Leave it out and it fixes things "while it is there".
   - **Conventions** — the repository's conventions that bear on this change (naming, how errors
     are returned, how comments are written, where and how tests are written). If there is an
     AGENTS.md, point to it.
   - **Tests** — the tests to add, by name and content. The command to run is the brief's `verify`.
   - **Commit / PR** — the commit message convention. The hub rewrites the PR body later, so tell
     Jules only "summarise the change briefly".
2. **Hand it over.**

   ```bash
   adj jules start --id {task_record} --prompt-file .claude/jules-prompt.md --base '{branch_name}'
   ```

   `{task_record}` is the brief's value. `{branch_name}` is the brief's "Base branch" line with
   `origin/` stripped (`origin/release/1.2` → `release/1.2`), the same rule §5 uses for the PR's
   base. Jules takes a branch name as it is on GitHub, so passing it with `origin/` points at a
   branch that does not exist. If the line is missing or `-`, do not guess; ask the user.
   **If the branch name contains anything other than letters, digits and `.` `_` `/` `-`, do not
   put it on the command line; ask the user.** The branching point may be a value a person typed
   on the board, and git allows `;` and the like in branch names. Even in single quotes, a `'`
   inside closes the quote and the rest runs as shell.
   The session id is written onto the board's card, and the card starts showing Jules's state.
   **If it fails, show the user why and stop.** Do not switch to implementing it yourself — who
   implements was decided by whoever created the task.
   If you are told there is no `julesKey`, or no item in the keychain, have the user register it as
   the message says. **Do not ask for the key or have it pasted into the conversation.**
3. **Report to the user.** The session's URL, and briefly what happens next (when Jules opens a PR
   the board moves the card to in review, and the hub rewrites the PR's description).
4. **Ask for cleanup with §9.** This worktree has no commits, so the unpushed check is quick.
   Under `## Result` write "handed to Jules session {id} ({url})".

Do not: implement, commit, open a PR, run a self-review, or comment on Jules's PR (review on the PR
is driven by a person).

## Appendix — Show a person and wait (gate)

**A gate is not "a question" but "something presented + handing the ball over".** Put what you
prepared on the board, end the turn, and wait. The answer arrives in the outbox.

### Opening one

Pass JSON to `adj gate open` on standard input. **Split the text into three slots**:

```bash
adj gate open --json <<'JSON'
{
  "kind": "plan",
  "task": "{the brief's Task record; leave it out if `-`}",
  "title": "Design review: caching search results",
  "problem": "The same search term hits the API every time, so every back navigation waits for results.",
  "goal": "Recently shown search results appear without waiting for the API.",
  "facts": ["6 files to touch", "base origin/release/1.2"],
  "focus": "Please decide how the TTL is held. Once that is settled I can start implementing.",
  "decided": "- An LRU of 64 entries, kept inside `SearchRepository`\n- Nothing is persisted",
  "unsure": "Not sure whether to keep the hit rate in metrics.",
  "choices": [
    { "id": "const", "label": "Option A — a constant", "why": "there is no remote config",
      "points": ["small diff (1 file)", "QA cannot change the value"], "recommended": true },
    { "id": "config", "label": "Option B — a setting", "why": "QA can change it",
      "points": ["medium diff (3 files)", "one more path for reading settings"] }
  ]
}
JSON
```

| Slot | What to write |
| --- | --- |
| `focus` | What **alone is enough to decide on**. The fork a person should decide |
| `decided` | What is settled. Folded on the board (need not be read) |
| `unsure` | Where you were not confident. **Do not swallow it** |

- `facts` are what is true whatever the decision (number of rounds, the `verify` result, lines).
- `choices` only **when you want an option picked**. Two options are shown side by side. Be honest
  with `recommended`.
- Extra fields per `kind`: `diff` (the diff as a string) / `run` (how to run it) / `body` (the
  report).
- `plan` has `problem` and `goal` (step 3 of §1).
- **Do not skimp on `title` and `focus`.** On the board they are all that gets read.

`diff` and `verify` carry structured fields apart from the text slots. The board builds its tables
from them. To keep it as a record, `"wait": false`; to wait, put the rules that stopped it in
`stoppedBy` (which one is decided by "Appendix — Wait or record").

```bash
adj gate open --json <<'JSON'
{
  "kind": "diff",
  "wait": false,
  "task": "{the brief's Task record; leave it out if `-`}",
  "title": "Diff review: caching search results",
  "facts": ["self-review converged in 2 rounds", "verify passed", "+120 / -8 lines"],
  "diff": "{the output of git diff}",
  "reviewRounds": [
    { "engine": "claude", "must": 1, "want": 1, "scope": 0, "falsePositives": 1 },
    { "engine": "claude", "must": 0, "want": 0, "scope": 0, "falsePositives": 1 }
  ],
  "findings": [
    { "severity": "must", "location": "src/search/cache.rs:42", "text": "old entries are not evicted past capacity",
      "outcome": "fixed" },
    { "severity": "must", "location": "src/search/cache.rs:10", "text": "read without taking the lock",
      "outcome": "declined", "reason": "the caller is single-threaded and it is not shared" },
    { "severity": "want", "location": "src/search/cache.rs:30", "text": "pull the capacity out into a constant",
      "outcome": "open" }
  ]
}
JSON
```

```bash
adj gate open --json <<'JSON'
{
  "kind": "verify",
  "stoppedBy": ["manual-check"],
  "task": "{the brief's Task record; leave it out if `-`}",
  "title": "Verification: caching search results",
  "focus": "- Please check that search results appear at once on back navigation",
  "run": "cargo run -- search 'cache'",
  "commands": [
    { "command": "cargo build", "result": "pass", "time": "12s" },
    { "command": "cargo test", "result": "pass", "time": "48s", "output": "test result: ok. 212 passed" }
  ],
  "manual": ["After searching and going back, results appear without waiting"]
}
JSON
```

- `reviewRounds` has one entry per round, with `engine` (`claude` / `codex`) and that round's
  counts of valid must / want / scope and false positives. It is not `rounds` (the number of round
  trips with a person); do not mix them.
- In `findings`, `severity` is `must` / `want` / `scope`, and `outcome` is `fixed` (fixed) /
  `declined` (rejected as a false positive; one line of reason in `reason`) / `open` (not fixed).
- In `commands`, `result` is `pass` / `fail`. `attempts` is the number of runs, only when it failed
  and passed after a fix. `manual` is the checklist a person goes through, one per line.

### After opening

`adj gate open` returns **`server: "up"` or `"down"`**. Branch on it:

- **A record opened with `"wait": false`** — whatever `server` says, **do not wait, and do not end
  the turn.** The reply carries `"wait": false` and "go on with your work", so go straight on to the
  next step. Even on `down`, do not ask with `AskUserQuestion` (it is a record because there is
  nothing to ask).
- **`up`** — say in one line that it is on the board and waiting for a decision, and **end the
  turn**. Do not poll. Do not `sleep`.
- **`down`** — the dashboard is not running. **Nobody will see it, so do not wait.** Ask with
  `AskUserQuestion` right away, in this tab as usual. (The gate's file remains, but as a record,
  not a place to meet.)

### When woken

**Start by reading `adj outbox`.** It is the only place answers arrive, and a decision comes in
this shape:

```
## Decision   approve | changes | reject | choice | ack | ask | answer
## Chosen     Option A — a constant (const)     ← only when choices were offered
## gate       {id} ({kind})                     ← ({kind}, record) when a record is sent back

## Comment

{what the person wrote, or (none)}
```

- `approve` / `ack`: go on to the next step.
- `changes` / `ask`: fix it as the comment says, and **if it is the same point, add one to
  `rounds`** and open it again. The board looks at that number and suggests to the person that
  talking in the tab would be faster.
- `choice`: implement the chosen option. **Do not reopen the question by bringing up the merits of
  the option not chosen.**
- A `changes` whose `## gate` line says `record` is **a record a person sent back after you had
  moved on**. Break off what you are doing, fix it as the comment says, and put the fixed diff
  through §3 again (opening a new gate or record). The record that was sent back stays as it is.
- **Do not close a gate yourself.** It is closed once the answer arrives.

### Do not

- **Do not move on to other things while waiting.** Implementing before approval only adds to what
  is thrown away if it is rejected.
- **Do not wait on `down`** (as above).
- **Do not pack two decisions into one gate.** People go through them one at a time from the top.

## Appendix — Wait or record (diff and verify)

`plan` **always waits**, whatever Stop at says. `diff` (§3) and `verify` (§4) wait if even one of
the rules below applies, and are kept as a `"wait": false` record and carried on past if none
does. **Either way, one of a gate or a record is always opened.** Carrying on without opening one
is not allowed.

| `stoppedBy` | When it applies | Applies to |
| --- | --- | --- |
| `round-limit` | Self-review reached `selfReviewRounds` with a valid must left | diff |
| `verify-failed` | `verify` failed and you could not fix it | diff / verify |
| `manual-check` | `manual` holds something only a person can check (a change on screen and the like) | verify |
| `unsure` | There is something to write in `unsure` | diff / verify |
| `stop-at` | The brief's Stop at includes this gate (`diff`: the diff; `all`: the diff and verify) | diff / verify |

- **When it waits, put every rule that applied in `stoppedBy`.** The board shows it as "why it
  stopped". A person looks at the reasons and decides whether it is theirs to look at.
- **Do not swallow things to keep `unsure` empty.** If you were unsure somewhere, write it and
  stop. Leaving it out because you want a record turns the rule inside out. When self-review
  stopped on "no progress" with a must left, or stopped early at the entry gate's "fix once" or
  "do not fix" with a must left, write the remaining must here as well. A record is never made
  while a valid must remains.
- **Do not stop for a reason that is not a rule.** "Just to be safe" is not a rule. Whether a person
  wants to look was decided by whoever handed the task over, in Stop at. Conversely, do not make a
  record when a rule applies.
- A record can still be read on the board and sent back with `changes` ("When woken").

## Appendix — Which agent does what

| Role | Who | Why there |
| --- | --- | --- |
| Research | A research agent (or the built-in `Explore` / `general-purpose`) | Dozens of files of exploration stay out of your context. Read-only |
| Implementation plan | The built-in `Plan` | Read-only. A person approves the plan, so it has to come back to this session |
| Implementation | **You** | You are already where the work belongs. Sub-agents only for mechanical bulk replacements and investigations whose tool output you do not want in your context |
| Self-review | A review agent (or `general-purpose`) / codex | Only an agent that does not know the implementation's reasoning avoids endorsing it. No `fork` |
| Handling review | A triage agent (or `general-purpose`) → you | Collecting and classifying is routine; deciding what to address needs a person |
| Talking to the user, reporting results | **This session** | Sub-agents cannot ask questions. Reports go to the user at this tab ("Where you stand") |
| Cleaning up the worktree | The hub (asked in §9) | You cannot remove the worktree you stand in |

---

## Appendix — The self-review loop (run until it converges)

Called from §3. One round = review → triage → sweep → fix. Run it **until it converges**. Do not stop
after one re-review.

### Severity vocabulary (shared by both engines)

Severity has these three levels. Whether a round goes to Claude or codex, **have it report in this
vocabulary** (say so in the prompt). If the vocabulary changes with the engine, the convergence check
below misfires on one side.

- **must** — a defect if merged. Wrong behaviour, a crash, data loss, security.
- **want** — correct, but could be better. Readability, reuse, an edge case that does not occur now.
- **scope** — a change the task did not ask for. A refactor on the side, formatting, leftover debug
  logging.

Only **must** decides convergence. want / scope are left in `findings` as `open`, and whoever reads
the diff gate or record decides whether to fix them (a record can be sent back with `changes`).

### Collecting what to review

What is reviewed is the current cwd. Plain `git …`, the reviewer's working directory is `.`, and
for codex `--cd .`. Apply fixes yourself (`Edit`). If you handed the implementation to another agent,
send it back to that id. **`isolation: worktree` is forbidden** — it digs another worktree under the
one you are standing in.

Triage, sweep, the convergence check and the final report always stay **with whoever is running the
loop**. They are not handed to the reviewer.

**Nothing is committed during the loop**, so `git diff {base}...HEAD` is empty. Do not review that.
Collect the diff as `git diff "$(git merge-base {base} HEAD)"` (tracked changes) and always add the
`??` lines of `git status --short` (untracked files) — new files are exactly the blind spot of a
reviewer who reads only the diff.

### Entry gate (ask once, not every round)

Run round 1 first, show the results in order of severity (High → Medium → Low), then:

- No valid must survived triage → already converged. Report and move on.
- Otherwise `AskUserQuestion` "Fix the findings and keep reviewing until it converges?"
  - **Keep going (Recommended)** → from then on, run to convergence **without asking each round**.
  - **Fix once** → fix once, re-review once, and stop.
  - **Do not fix** → skip and move on.
  - In either of the last two, if a valid must is left, stop the diff gate in §3 on `unsure`
    ("Appendix — Wait or record").

### One round

1. **Review** — pick the engine for the round ("Choosing the review engine" below) and have it
   review the diff. **Report only**; do not let the reviewer touch files.
   - How to collect the diff and the reviewer's working directory are as in "Collecting what to
     review" above.
   - **Carry the false-positive list into every round's prompt** (the rejected findings + a
     one-line reason each). Add "these were checked against the real code and judged false
     positives, so do not raise them again". This is what matters most: without it the reviewer
     raises the same false positives forever and the loop never converges.
   - Also pass what was fixed in the latest round, so it reviews the current state rather than the
     history.
2. **Triage — always on the side running the loop.** Do not let the reviewer certify its own
   findings. Check each finding against the real code before touching it (the same principle as
   with Copilot: do not take it at face value).
   - **Valid must** → into this round's set of fixes.
   - **False positive** → onto the false-positive list with a one-line reason.
   - **want / scope** → recorded but not counted for convergence. A person decides whether to fix
     them, as in "Severity vocabulary" above.
3. **Sweep** — for each valid finding, grep the whole diff for **the kind of defect**, not the
   reported line, and fold what turns up into the same set of fixes. The reviewer only saw what it
   looked at, and the same drift is often sitting in a file it skipped.
4. **Fix** — fix the round's valid findings **all at once** (do not run a fix per finding).
5. On to the next round.

### Convergence / stopping conditions

Stop when any one holds:

- **Converged**: **zero valid must** in the round — whether it was "no findings" or everything fell
  to false positive / want / scope in triage. This is the goal (valid findings have run out and
  only wrong ones come back). "No findings" is merely the easy case.
- **Round limit**: `selfReviewRounds` (default 5). Report "stopped after N rounds" and what remains.
  If a valid must remains, stop the diff gate in §3 on `round-limit` and ask there whether to go on
  (put the remaining must in `findings` as `open`).
- **No progress**: a valid finding survived your own fix round untouched, or two findings started
  going back and forth (fixing A brings B back, and vice versa). Another round will not help. Write
  the remaining must in `unsure`, and stop the diff gate in §3 on `unsure` to hand it to the user.

### Final report

Per round, compactly:

```
Self-review: converged in 3 rounds (engine: R1-2 Claude / R3 codex)
  R1: must 3 → fixed / false positive 1
  R2: must 1 → fixed / false positive 2
  R3: must 0 (false positive 2 / want 1) → converged
  Rejected as false positive: <finding> → <reason>
  Open want / scope: <finding>
```

The reasons for false positives can be reused. Offer to save them to lk (`lk add … --scope user`).
Later reviews, and codex reading lk directly, stop raising the same false positives.

### Choosing the review engine (switching on 5h / 7d headroom)

The loop eats tokens, so when either of Claude's rate limits (the 5-hour window or the 7-day window)
runs low, send **only the review** to codex. Follow the `reviewEngine` setting (default `auto`).
**Check again at the start of every round** — usage keeps rising while the loop runs.

- `reviewEngine: "codex"` → follow **Running a codex review** below.
- `reviewEngine: "claude"` → follow **Running a Claude review** below.
- `reviewEngine: "auto"` (default) → decide the engine with the rate-limit check below:
  1. Read the cache `statusline.py` writes on every draw. **It lives per account**, directly under
     this session's config directory (so as not to pick up another account's headroom, such as a
     work one):
     ```bash
     cat "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/rate-limit-cache.json"
     ```
     Shape: `{"captured_at": <epoch>, "five_hour": {"used_percentage": 42.3, "resets_at": <epoch>}, "seven_day": {...}}`
  2. Decide on `five_hour.used_percentage` and `seven_day.used_percentage`. If **either** trips,
     codex:
     - `five_hour >= 50` **or** `seven_day > 70` → say once which window tripped and **that
       window's own** `resets_at` (if both trip, name 5h): "{5h|7d} is at {pct}%, so the review
       switches to codex (resets {resets_at})", then follow **Running a codex review** below.
     - The two keys are evaluated **independently**. A missing `seven_day` does not stop the 5h
       check, and vice versa.
     - The cache is missing / broken / **both** keys are missing / `captured_at` is older than 15
       minutes → unknown. Tell the user the usage check was skipped, and go on with **Running a
       Claude review** below.
     - `which codex` fails → tell the user, and go on with **Running a Claude review** below.
     - Neither trips → follow **Running a Claude review** below.

#### Running a codex review

```bash
codex exec --sandbox read-only --cd . --add-dir ~/.config/lk - < {prompt file}
```
`{prompt file}` is the path of a temporary file you wrote out. The prompt is long and full of
quotes, so pass it on standard input rather than as an argument.
**The sandbox is `read-only`.** You are asking for "report only, do not change the code", so there
is no reason to hand over write access. Making it unable to touch anything is surer than relying on
the request and noticing afterwards in `git status` (a reviewer that fixes things is not an
independent review but an endorsement of your own implementation).
Say "report only, do not change the code" in the prompt, and **that severity must come back as
must / want / scope** ("Severity vocabulary" above). When it is done, check that the worktree has
not changed, to be safe (`git status --short`).

#### Running a Claude review

Check the description of the `Agent` tool; leaving out `general-purpose` and triage agents (whose
description or name mentions triage), if there is an agent that specialises in code review and
checking diffs (its description mentions diffs, self-review or code review, or it is something like
`self-reviewer`), pick it. If there is none, pick `subagent_type: "general-purpose"`.
Either way, tell it "Do not use any tool that changes files or runs commands, such as Write / Edit /
Bash; only check the diff with Read / Grep / Glob and report a review" and "always return severity
as must / want / scope", and run it (pass `reviewEffort`).
- Whichever engine reviewed, **triage, sweep, the convergence check and the fixes stay with you**.
  Choosing the engine only changes "who reads the diff".
