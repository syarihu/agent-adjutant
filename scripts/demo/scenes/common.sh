# Helpers for the scene setups, which run after start.sh: they put the board in the state a clip
# starts from, through adj as the hub would (a task, a worktree, a worker in it).
source "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
cd "$DEMO_ROOT/src/shop-app"

# start_worker NAME TITLE KIND BODY: a task, its worktree and a scripted worker playing NAME.
start_worker() {
  local name="$1" title="$2" kind="$3" body="$4" wt="$DEMO_ROOT/src/shop-app/.claude/worktrees/$1" task
  task=$(adj task add --title "$title" --body "$body" --kind "$kind" --done-when "${DONE_WHEN:-pr}" \
    ${ISSUE_URL:+--issue-url "$ISSUE_URL"} --worktree-name "$name" --json |
    python3 -c 'import json,sys;d=json.load(sys.stdin);print((d.get("task") or d)["id"])')
  git worktree add -q -b "demo/$name" "$wt" main
  adj task brief --id "$task" --worktree "$wt" --base main > /dev/null
  adj task update --id "$task" --status dispatched --worktree "$wt" > /dev/null
  adj work --worktree "$wt" --task "$task" > /dev/null
  echo "$task"
}

# add_task TITLE ISSUE_NUMBER STATUS [PR_URL] [BODY]: a task with no worker.
add_task() {
  local title="$1" issue="$2" status="$3" pr="${4:-}" body="${5:-${BODY:-$title。}}" task
  task=$(adj task add --title "$title" --body "$body" \
    --issue-url "https://github.com/acme/shop-app/issues/$issue" \
    --worktree-name "task-$issue" --json |
    python3 -c 'import json,sys;d=json.load(sys.stdin);print((d.get("task") or d)["id"])')
  adj task update --id "$task" --status "$status" ${pr:+--pr "$pr"} > /dev/null
  echo "$task"
}

# board_post PATH [JSON]: a POST to the resident server, as the page makes one.
board_post() {
  local port; port=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["port"])' "$ADJUTANT_STATE_DIR/server.json")
  curl -s -X POST -H "X-Adjutant-Token: $(cat "$ADJUTANT_STATE_DIR/dashboard-token")" -H "Origin: http://127.0.0.1:$port" \
    -H 'Content-Type: application/json' "http://127.0.0.1:$port$1" -d "${2:-{\}}"
}
