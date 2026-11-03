#!/usr/bin/env bash
# The scripted worker. Which scene it plays is told by its worktree's name (scenes.sh).
SESSION_ID="$1"; WORKTREE="$2"; TITLE="$3"
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/scenes.sh"
cd "$WORKTREE"
WHERE="~/src/shop-app/.claude/worktrees/$(basename "$WORKTREE")"
hook SessionStart
clear
banner "worker · $TITLE"
user "Read .claude/task-brief.md and work the task as it says."
worker_scene "$(basename "$WORKTREE")"
while :; do idle_prompt; read_typed typed; user "$typed"; worker_woken "$(basename "$WORKTREE")"; done
