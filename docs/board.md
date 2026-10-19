English | [日本語](board.ja.md)

# Web Board and Resident Server

agent-adjutant provides a web board to monitor tasks, manage sessions, inspect diffs, and answer agent gates across repositories.

---

## Overview and Serving Modes

The board can be served in two ways:

1. **Resident server (`adj server start`) [Recommended]**:
   A single background server process per state directory that serves all repositories at `/b/<slug>/` and provides a unified dashboard at `/`.
2. **Repository hub board (`adjutant serve` or automatic `hubServe`)**:
   Served directly by a repository's hub MCP server for the duration of that hub's lifetime (defaults to port 4577 or the next free port). Only displays tasks and workers for that specific repository.

### Resident Server Management

```bash
adj server start [--port <port>] [--foreground] [--no-open]
adj server status [--json]
adj server stop
adj server restart [--port <port>] [--open]
```

- **Port allocation**: Defaults to `127.0.0.1:4577`, or any free port if 4577 is occupied.
- **Switching repositories**: On the resident server, `/` opens 「いまの仕事」; the rail's ボード entry lists all repositories and parent-task hubs in the column beside it, and selecting "すべて" (`/?view=agent`) displays tasks across all boards.
- **Automatic reconnection**: Existing browser tabs automatically reconnect if the port remains unchanged after `adj server restart`.
- **Security & Tokens**: Bound strictly to `127.0.0.1`. Every request needs the token stored in `~/.local/state/adjutant/dashboard-token`. The page reads it from the `token` URL query it was opened with and sends it in the `X-Adjutant-Token` header; state-changing requests are refused unless the token comes in that header. Separately, state-changing requests and terminal WebSocket connections must carry a browser `Origin` header that matches the board's own origin.

### Keeping the Resident Server Running with launchd (macOS)

To start the resident server automatically across logins, create a LaunchAgent at `~/Library/LaunchAgents/adj.server.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>adj.server</string>
  <key>ProgramArguments</key>
  <array>
    <string>/path/to/adj</string>
    <string>server</string>
    <string>start</string>
    <string>--foreground</string>
    <string>--no-open</string>
  </array>
  <key>KeepAlive</key><true/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin</string>
  </dict>
</dict>
</plist>
```

- Load: `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/adj.server.plist`
- Unload: `launchctl bootout gui/$(id -u)/adj.server`
- Restart: `launchctl kickstart -k gui/$(id -u)/adj.server` (Do not run `adj server restart` under `KeepAlive`, as it can race with launchd).

---

## Views

The page is a rail of icons with the list of the place chosen in a column beside it. The rail has **いまの仕事 (Work under way)**, **TODO**, **レビュー (Review)** and **ボード (Board)**, with the organisation switch and a ＋ button (「依頼する」) above them and the notification settings and the theme below. `/` opens いまの仕事 on the resident server; `/?view=agent` is 「すべて」, and a board's own address (`/b/<slug>/`) opens that board. The list column's width is dragged between 240px and 560px and remembered. A board served alone has the rail and the board only.

### 1. いまの仕事 (Work Under Way: `?view=work`)

A unified 3-column operational view across all repositories:
- **Left Column**: The list of active sessions, hubs, gates, and review requests.
- **Middle Column**: The embedded tmux terminal of the currently selected session or hub, under a pinned heading: the title, the branch and agent, the Issue and PR links, and buttons at the right end (「タスク詳細」, 「IDE」 and 「セッションを閉じる」 for a task's worker; 「hub の詳細」 for a hub). Everything below is laid over the terminal and takes no room from it, so a question coming or going never resizes the terminal.
  - **Questions** slide down over the terminal's top. A quick confirmation (`quick`) is answered in its line; any other is one line with 「開いて答える」, which opens the question card. A hub's start check (a `dispatch` question that is not quick) is a form over the top half of the terminal, with its answer buttons always in view; it folds to one line.
  - **Two buttons at the bottom right.** 「最後のメッセージ」 shows the agent's last message as rendered Markdown (the whole text when the board still has it). 「書いて送る」 opens a wide box; its text is pasted into the terminal as one paste, and 「送ったら Enter も押す」 presses Enter after it (⌘Enter or Ctrl+Enter sends). The draft is kept per session while the page is open. The text goes in as one paste: an agent that reads bracketed paste (Claude Code does) gets it as one input, while a program that does not gets the lines as if typed.
  - **A hub** has no right column here: its status and counts are one line in the heading, and 「hub の詳細」 opens the rest (queue, inbox, idle worktrees, actions) in a dialog. A stopped hub shows 「hub を起動」 over its terminal (its start check, if any, is one line above it); while the start request is out, the block says 「起動しています…」 and has no button.
- **Right Column**: The task detail panel, aide pane, or AI review summary. Not shown for a hub.

#### Organisation, repository and hierarchy
The left list shows one organisation (a GitHub owner) at a time:
- **Organisation switch**: the avatar at the top of the rail. Its menu lists every owner of a registered repository, with how many things wait in each; the badge on the avatar adds up the other organisations'. Following a link to a row of another organisation switches to it.
- **Repository filter**: 「すべて」 or one repository of the organisation. With one chosen, the repository headings are left out.
- **Hierarchy**: Repository → hub → sessions. A hub is a row heading its group; the tasks it runs are indented below it. A parent issue's hub shows the parent's number, title and `n / m マージ` progress.
- **依頼する**: opens one dialog to ask a hub for a task: pick the repository (of the organisation) and the hub (the repository's own, or a parent task's), write what you want, and send it with 「渡す」 (Ctrl/⌘+Enter) or keep it with 「あとで（Backlog）」. An unsent text is kept for the next time. The new-task button and 「子タスクを足す」 open the same dialog; a board served alone keeps its own form.
- **未読の依頼**: a hub row says how many requests it has not read, with the newest subject; a hub that is not running offers 「hub を起動」.
- **片付ける**: a row whose PR is merged stays listed after its worker exits. Its last line gives the uncommitted files and unpushed commits that removing the worktree would lose (`確認しています…` while it reads, and no button if git cannot be read). Only when there is nothing to lose does it offer 「片付ける」; one press removes the worktree and its branch. If work turned up in between, nothing is removed and the cleanup dialog opens for the forced path.

#### 新着 (New / Attention Required)
Holds items waiting for your action:
- A newly opened gate
- A session waiting on a permission prompt (`waiting`) or user question (`AskUserQuestion`)
- A completed turn with no gate open
- A failed session
- A PR requiring your attention (changes requested, CI failed, approved, closed without merging)

Items leave 新着 when acted upon (answering the gate, typing in the terminal, or clicking `done` / `done_all`).

#### 離れていた間に (While You Were Away)
Shows a summary block at the top of a task panel detailing events that occurred since you last interacted with or viewed that row (e.g. phase transitions, gates opened, permission waits, turn completions).

#### Parking a Task (タスクを置く)
Temporarily set aside a task while waiting on external feedback (e.g. PdM, designer, external review, or merge timing):
- CLI: `adj task park --id <id> --reason pdm|design|review|merge-timing|other [--text <text>]`
- CLI: `adj task unpark --id <id>`
- UI: Click "置く" in the task summary.
Parked tasks do not appear in 新着 and do not ring desktop notifications.

### 2. AI 下読み (AI Pre-reading / PR Review Assistant)

Switches the left list to review requests from other people across repositories:
- PR metadata: size (S / M / L), diff overview, CI status, reviewer turn.
- Pre-read breakdown: Reading order, key review points, structural breakdown (specs/logic, UI, docs, tests, resources).
- Draft comments: Edit review drafts directly within the board and push them in batch to your pending review on GitHub via "N 件を pending review に積む".
- Oversized PRs (>1,500 lines changed in a single file) offer "分割案" (split proposal) to suggest splitting the PR before reading.

#### Code-Review Hub and Readers
- Start review hub: `adj hub --review` (or click "hub を起動" on the review hub row).
- Request a read: `adj review request --pr <URL|owner/repo#N>` (or click on the PR row).
- Concurrent readers: Up to 3 run concurrently in isolated temporary directories without mutating repository state.

### 3. 副官 (The Aide)

An interactive assistant session (shown when no specific task is selected in 「いまの仕事」):
- Start/reset via `POST /api/aide/open` or clicking "副官に聞く".
- Runs Claude Code in `<state>/aide` using your own global configuration, augmented with read-only tools, `adjutant_show`, and two action tools: `adjutant_request` (delegates work requests to a hub) and `adjutant_gate_answer` (answers gates on your behalf).
- **Display Area (`adjutant_show`)**: The aide can render gates, wait lists, PR pre-reads, diff snippets, active requests (`request`), and task statuses directly into the adjacent display pane.
- **Answering gates through the aide**: Clicking 「副官に答えを伝える」 on a waiting gate or 「この判断」 pre-fills `[gate <board>/<id>] ` in the prompt. The aide reads your instruction (e.g. 「B で」), previews the proposed decision, and answers the gate once you confirm.

---

### 4. TODO (`?view=todo`)

What waits on you, across every repository. The rail's TODO entry carries a badge with the count; the count is every open gate that is not parked plus every pull request you were asked to review, whether or not its AI pre-reading has started. Terminal-only permission prompts, parked tasks' gates and your own tasks' PR turns are not in it.
- **Popover**: resting the pointer on the entry (or focusing it with the keyboard) opens a list beside the rail. A quick confirmation is answered right there; any other row opens the TODO view with that item selected. It closes when the pointer and focus have left it, or on Escape, and never opens in the TODO view or over a dialog.
- **Left**: 「あなたの番」, in the order it is best answered. First the quick confirmations, oldest first. Then 「読んで決める」: start checks, filing confirmations, research results and reports, plans, questions, what is to go to Jules, manual checks and diffs, oldest first within each kind. Then the pull requests you were asked to review, with how far their AI pre-reading has got.
- **Right**: the selected question as the same card the dialog shows (answered where it stands, with the diff, the Issue body and the report opening over it), or for a pull request a short card with a button into its review. With nothing waiting it says 「あなたの番はありません」.

An answered item leaves the list at once. The selected item is page state; only `?view=todo` is in the address. The list is narrowed in one place, `todoInScope` in `src/ui/todo-view.js`.

## Task Panel & Gates

Clicking any task card opens its detail panel:

### Panel Tabs
1. **タスクサマリ (Task Summary)**: Issue and PR references, active gates, phase progress, problem description, goal, and worktree info.
2. **ターミナル (Terminal)**: Live embedded terminal for the worker session (requires `terminal.preset: "tmux"`).
3. **コードレビュー (Code Review)**: Review rounds, findings, diffs, and verification records.
4. **動作確認 (Verification)**: Verification commands, manual check checklists, and results.
5. **経過 (History)**: Chronological log of past gates, decisions, and phase updates.

### Gates

Gates represent explicit decision points where an agent stops and waits for human input:
- **Frame structure**: What to look at, what was already decided, and where agent confidence is uncertain. A gate that waits always carries the question to answer (`focus`); `adj gate open` refuses one without it.
- **What a gate carries**: `steps` (a plan's steps, in order; required on a `plan`), `sections` (what was found or planned, as blocks under headings the agent picks, in the `found` or `plan` stage) and `quick` (a fixed confirmation answered with one press, with nothing to read).
- **Gate Types**:
  - `plan`: Approval of implementation plan, problem statement, and goals.
  - `diff`: Code review rounds and findings.
  - `verify`: Verification commands and manual test checks.
  - `dispatch`: Task confirmation prior to worker launch.
  - `result`: An investigation's report, read rather than approved. Three answers: got it, ask more, or make it a task.
- **Answering**: Choose an option (Approve, Revise, etc.), add optional comments, or answer directly in the session terminal (`adj gate close --terminal`).
- **The question card** (opened from the line over a task's terminal in 「いまの仕事」; the task panel's tabs and 「拡大」 keep their own answer area): one dialog that reads top to bottom: あなたの依頼 (the request in your words and the goal), 調べたこと, 計画 (計画とやったこと on a diff or verify gate: the steps, marked done, the places to look at, open and declined findings, 懸念点), 前に決まっていること (only when it bears on this question), then the question itself as the heading, the choices one to a row, the comment box and the buttons. A part with nothing in it is not drawn, and the question is written once. The header and the answer area stay put; only the middle scrolls. A card has links only to what it lacks: 「差分を全部見る」 on a diff gate, 「Issue の本文」, 「レポートを開く」.
- **Dialogs stack**: what the card opens (the full diff, the Issue body, a report) goes over it as a further layer. 「←」 or Esc goes back one layer; × closes them all. What you typed in the card's comment box is kept while the page is open.
- **Gates written before steps and sections existed** use the same four parts: the request is the task's body (else the Issue's title, with its body one layer away), what was found is the gate's `problem` and, for a result, its `body`, a plan with no `steps` shows `decided` as the plan, and a gate with no `focus` asks the fixed question of its kind (or its title).
- **Server status detection**: `adj gate open` checks whether a board server is active (`server: up` / `server: down`). If no server is running, the agent falls back to asking directly in its terminal.

---

## Card Placement & GitHub Sync

Cards automatically advance across board columns based on worker phases and GitHub PR states:

| PR State | Whose Turn | Board Column |
| --- | --- | --- |
| Draft | Worker | In Progress (Agents) |
| Ready, assigned to other reviewers | Other reviewer | Agents column (レビュー待ち（他の人）) |
| Waiting on review bots or CI | Bots / CI | Agents column (bot・CI 待ち) |
| Changes requested | You | PR Review (修正の依頼あり) |
| Approved | You (to merge) | PR Review (マージ待ち) |
| Failing CI | You | PR Review (CI 失敗) |
| Merged | Nobody | Done |
| Closed without merge | You (decision) | PR Review (閉じられた) |

The resident server polls GitHub notification events (`GET /notifications`) with `If-Modified-Since` and queries updated PRs in single GraphQL batches. Cards refresh every 5 minutes as a fallback. Manual refresh is available via `adjutant task refresh` or clicking "PR確認".

---

## Working with Sessions

### Sessions without a Task
You can start a freeform worker session without pre-creating an issue or task:
- UI: Click "タスクなしのセッションを始める…" on the hub panel.
- API: `POST /api/sessions` with `{"instruction": "...", "worktreeName": "..."}`.
- Later link to a task via `POST /api/sessions/<id>/link`.

### Session Lifecycle Actions
From the terminal action bar or API:
- **Resume (`POST /api/sessions/<id>/resume`)**: Reopen a stopped worker in its existing worktree.
- **Restart (`POST /api/sessions/<id>/restart`)**: Recycle the worker window while preserving session context.
- **Open in External Terminal (`POST /api/sessions/<id>/open`)**: Attach to the tmux session using your native terminal (e.g. iTerm2 via `tmux -CC attach`).
- **Clean Up (`POST /api/sessions/<id>/cleanup`)**: Remove the worktree and local branch if changes are fully committed and pushed. Requires confirmation (`force: true` with directory name) if untracked/uncommitted changes exist.
