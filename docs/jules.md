English | [日本語](jules.ja.md)

# Handing Tasks to Jules

A task can be implemented by [Jules](https://jules.google) instead of a local worker. No local worker session is launched: the hub cuts a detached worktree, has a subagent write a comprehensive design plan, and once a human approves the plan, hands the task over to a Jules session.

---

## Prerequisites

1. **Jules GitHub App**: Ensure the Jules GitHub App is installed on the target repository.
2. **API Key Setup**:
   The `julesKey` setting points to a command that prints your API key. On macOS, store the key in the keychain item `jules-api`:

   ```bash
   security add-generic-password -s jules-api -a "$USER" -w
   ```

   The key is piped to `curl` over stdin and masked in logs and command outputs. On Linux, configure `julesKey` in your configuration to a command that outputs the secret.

---

## Workflow

### 1. Marking a Task for Jules

Specify `--executor jules` when creating or updating a task:

```bash
adjutant task add --executor jules
# or
adjutant task update --id <task-id> --executor jules
```

### 2. Planning via Subagent

For a Jules task, the `adj-hub` procedure delegates planning to a planning subagent. The subagent writes a concrete, detailed implementation design (`design.md`):
- Lists all affected files and the exact edits required.
- Explicitly notes files that must not be touched.
- Details test cases and verification steps.

The hub opens a `plan` gate using `adj gate open --body-file design.md --openedBy hub`. The board displays the design document directly for human review.

### 3. Launching the Session

Once the human approves the plan, the hub dispatches the task to Jules and disposes of the temporary worktree:

```bash
adj jules start --id <task-id> --prompt-file design.md
```

- Starts the Jules session from the branch specified by `--base` (or the task's base).
- Instructs Jules to open a PR automatically upon completion.
- Records the session ID under `julesSession` on the task record.
- You can query session progress with `adj jules show --id <task-id>`.

---

## Board Tracking & PR Handling

On the board, a Jules card displays the session state (queued, working, done, failed) linked to the `jules.google.com` dashboard:
- The card is not marked as stuck if no local worker is running.
- When the Jules session opens a pull request, the board captures the PR URL, updates the task record, and moves the card to review.
- The hub procedure rewrites the generated pull request description to match repository style guidelines.

---

## Review Comments and Relay

Jules responds only to comments posted by the GitHub account that initiated the session and ignores threads started by other bots or reviewers.

### Manual Review Relay
The task panel provides **レビュー指摘を Jules に回す**:
1. Aggregates the first comment of review threads left by reviewers, Copilot, or review bots.
2. You select which findings to forward.
3. Posted as a single consolidated comment under the account `gh` is signed in as. When the task records who started the Jules session, that account must be the starter, or the relay is refused; switch with `gh auth switch` and retry.
4. CLI equivalents:
   ```bash
   adj jules findings --id <task-id>
   adj jules relay --id <task-id> --comment <comment-id>
   ```

### Automated Relay Triage via Hub
While the board is running:
1. When new comments arrive from external reviewers while Jules is idle, the board sends a `jules-review` message to the hub.
2. The hub uses a subagent to evaluate each comment against the code without checking out the branch.
3. The hub opens a `relay` gate presenting filtered findings, suggested adjustments, and rationale for excluded comments.
4. Once approved, comments are posted automatically with `adj jules relay --plan-file`.
5. This automated triage runs at most twice per PR (`relayRounds`).
