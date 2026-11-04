# The scenes the scripted agents play. A request's words pick the scene; a worktree's name tells a
# worker which one it is in. Every record is made through adj, as a real hub and worker make it.

field() { printf '%s\n' "$2" | sed -n "s/^## $1 *//p" | head -1; }
gate_open() { adj gate open --json > "$DEMO_ROOT/gate-$1.out" 2>&1; }
scene_state() { echo "$2" > "$DEMO_ROOT/scene-$1"; }
scene_get() { cat "$DEMO_ROOT/scene-$1" 2>/dev/null; }

# ── The hub ──────────────────────────────────────────────────────────────────────────────────

hub_message() { # NAME KIND SUBJECT BODY
  local name="$1" kind="$2" subject="$3" body="$4"
  tool Bash "adj pending --read $name" "$subject"
  case "$kind" in
    request) hub_request "$(field task "$body")" "$body" ;;
    gate) hub_gate_answer "$body" ;;
    *) say "受け取りました: $subject" ;;
  esac
}

hub_request() { # TASK BODY
  local task="$1" body="$2"
  case "$body" in
    *クーポン*) hub_read_coupon "$task" ;;
    *) say "依頼を読みました。" ; stop "依頼を読みました。" ;;
  esac
}

hub_read_coupon() {
  local task="$1"
  scene_state coupon-task "$task"
  say "依頼を読みました。クーポンの有効期限の判定を直す作業として受けます。"
  tool Grep "expiresAt" "src/cart/coupon.ts:4, src/cart/coupon.ts:8"
  adj task update --id "$task" --title "有効期限の当日にクーポンが使えない" --worktree-name coupon-expiry > /dev/null
  tool Bash "adj task update --id $task --title …" "有効期限の当日にクーポンが使えない"
  if ! gate_open dispatch <<JSON
{
  "kind": "dispatch",
  "task": "$task",
  "title": "着手確認: 有効期限の当日にクーポンが使えない",
  "focus": "依頼をこう読みました。この内容で着手してよいですか？",
  "facts": ["Issue: なし（起票せずに着手）", "Base: main"],
  "decided": "- やること: 有効期限の当日にもクーポンを適用する\n- 完了条件: PR を出すまで\n- 止める所: 計画の承認\n- 実装: worker\n- worktree 名: coupon-expiry"
}
JSON
  then
    stop "着手前の確認を出せませんでした。"
    return 1
  fi
  tool Bash "adj gate open (着手確認)" "server: up · ボードで確認を待っています"
  say "着手前の確認をボードに出しました。答えが来たら worker を起動します。"
  stop "着手前の確認をボードに出しました。答えが来たら worker を起動します。"
}

hub_gate_answer() { # BODY
  local body="$1" task
  task=$(scene_get coupon-task)
  case "$body" in
    *approve*|*着手する*) hub_start_coupon "$task" ;;
    *) say "答えを受け取りました。"; stop "答えを受け取りました。" ;;
  esac
}

hub_start_coupon() {
  local task="$1" wt="$PWD/.claude/worktrees/coupon-expiry"
  say "着手の承認が来ました。worker を起動します。"
  adj task update --id "$task" --read --auto-start true > /dev/null 2>&1
  git worktree add -q -b demo/coupon-expiry "$wt" main
  tool Bash "git worktree add -b demo/coupon-expiry .claude/worktrees/coupon-expiry" "Preparing worktree (new branch 'demo/coupon-expiry')"
  adj task brief --id "$task" --worktree "$wt" --base main > /dev/null 2>&1
  adj task update --id "$task" --status dispatched --worktree "$wt" > /dev/null 2>&1
  adj work --worktree "$wt" --task "$task" > /dev/null 2>&1
  tool Bash "adj work --worktree .claude/worktrees/coupon-expiry --task $task" "started in a new tab"
  say "worker を起動しました。計画ができたらボードに確認が届きます。"
  stop "worker を起動しました。計画ができたらボードに確認が届きます。"
}

# A clip says when the next beat may start by creating the file this waits for.
wait_go() { while [ ! -f "$DEMO_ROOT/go-$1" ]; do sleep 0.2; done; rm -f "$DEMO_ROOT/go-$1"; }
task_of_worktree() { python3 -c 'import json;print(json.load(open(".claude/adjutant-worker.json")).get("task",""))' 2>/dev/null; }

# ── The worker ───────────────────────────────────────────────────────────────────────────────

worker_scene() { # WORKTREE-NAME
  case "$1" in
    coupon-expiry) worker_plan_coupon ;;
    checkout-slow) worker_investigate_checkout ;;
    checkout-batch) worker_pr_checkout ;;
  esac
}

worker_plan_coupon() {
  adj phase --set plan > /dev/null
  working "brief を読んでいます"
  tool Read ".claude/task-brief.md" "Read 42 lines"
  tool Read "src/cart/coupon.ts" "Read 12 lines"
  tool Grep "applyCoupon" "src/cart/cart.ts:15"
  say "原因が分かりました。期限を時刻まで比べているので、当日の 0 時を過ぎるとクーポンが使えなくなります。"
  working "計画を書いています"
  local task; task=$(python3 -c 'import json;print(json.load(open(".claude/adjutant-worker.json")).get("task",""))' 2>/dev/null)
  gate_open plan <<JSON
{
  "kind": "plan",
  "task": "$task",
  "title": "計画: 有効期限の当日にクーポンが使えない",
  "problem": "applyCoupon が expiresAt を時刻まで比べているため、有効期限の当日でも 0 時を過ぎると割引が外れる。",
  "goal": "有効期限の日の終わりまでクーポンが使える。",
  "facts": ["触るファイル 2 つ", "base: main"],
  "focus": "期限をどの時刻で区切るかを決めてください。決まれば実装に入ります。",
  "steps": [
    { "text": "期限を日付の終わりで比べる関数を足す", "detail": "src/cart/coupon.ts" },
    { "text": "applyCoupon をその関数で判定する", "detail": "src/cart/coupon.ts" },
    { "text": "当日・翌日・タイムゾーン境界のテストを足す", "detail": "src/cart/coupon.test.ts" }
  ],
  "decided": "- 割引率の計算は変えない",
  "unsure": "ショップのタイムゾーンを設定から読むべきかは確かめていません。",
  "choices": [
    { "id": "jst", "label": "A. JST の日付で区切る", "why": "ショップが日本向けだけ", "points": ["差分が小さい（1 ファイル）", "海外向けを始めたら直しが要る"], "recommended": true,
      "comment": "A で進めてください。タイムゾーンは定数にまとめておいてください。" },
    { "id": "shop-tz", "label": "B. ショップ設定のタイムゾーンで区切る", "why": "海外向けにも効く", "points": ["差分は中くらい（3 ファイル）", "設定を読む経路が 1 つ増える"] }
  ]
}
JSON
  tool Bash "adj gate open (計画)" "server: up · ボードで承認を待っています"
  say "計画の確認をボードに出しました。承認を待っています。"
  stop "計画の確認をボードに出しました。承認を待っています。"
}

worker_woken() { # WORKTREE-NAME
  adj outbox > "$DEMO_ROOT/outbox-$1.md" 2>&1
  tool Bash "adj outbox" "$(grep -m1 -E 'jst|shop-tz|approve|A\.|B\.' "$DEMO_ROOT/outbox-$1.md" | cut -c1-60)"
  adj outbox --clear > /dev/null 2>&1
  case "$1" in
    coupon-expiry)
      say "承認されました。JST の日付で区切る方針で実装します。"
      adj phase --set implement > /dev/null
      tool Edit "src/cart/coupon.ts" "Added 6 lines, removed 1 line"
      working "テストを書いています"
      ;;
    checkout-batch)
      say "下読みの指摘を受け取りました。在庫の問い合わせを 50 件ずつに分けます。"
      adj phase --set implement > /dev/null
      tool Read "src/catalog/stock.ts" "Read 15 lines"
      tool Edit "src/catalog/stock.ts" "Added 8 lines, removed 3 lines"
      working "テストを足しています" 2
      ;;
    checkout-slow)
      say "報告を受け取ってもらえました。調査はここまでです。"
      stop "報告を受け取ってもらえました。調査はここまでです。"
      ;;
  esac
}

worker_investigate_checkout() {
  adj phase --set plan > /dev/null
  tool Read ".claude/task-brief.md" "Read 38 lines"
  wait_go checkout
  tool Read "src/checkout/checkout.ts" "Read 12 lines"
  working "チェックアウトの処理を追っています" 1.5
  tool Grep "fetchStock" "src/checkout/checkout.ts:6, src/catalog/stock.ts:1"
  say "在庫の確認が商品ごとに 1 件ずつ、順番に API を呼んでいます。"
  working "ステージングで計測しています" 2.5
  tool Bash "node scripts/measure-checkout.js --runs 20" "median 3,074 ms (stock 2,480 ms)"
  cp "$DEMO_SRC/assets/checkout-report.html" .claude/report.html
  adj phase --set report > /dev/null
  gate_open result <<JSON
{
  "kind": "result",
  "task": "$(task_of_worktree)",
  "title": "調査報告: チェックアウトが遅い原因",
  "focus": "原因はこれで分かりましたか？直すなら、在庫 API をまとめて問い合わせる形にするのがよさそうです。",
  "facts": ["ステージングで 20 回計測", "カート 8 点で中央値 3.1 秒"],
  "sections": [
    { "heading": "結論", "body": "遅さの 8 割は在庫の確認です。カートの商品ごとに在庫 API を **1 件ずつ順番に** 呼んでいて、8 点なら 2.5 秒かかります。" },
    { "heading": "原因", "body": "\`checkout()\` が \`for\` の中で \`await fetchStock(sku)\` しているため、問い合わせが並ばず直列になっています（\`src/checkout/checkout.ts:6\`）。" },
    { "heading": "対応策", "body": "- 在庫 API の一括問い合わせ（\`/api/stock?skus=\`）を使う\n- 8 点でも 0.35 秒ほどに縮む見込み", "stage": "plan" }
  ],
  "unsure": "本番のレイテンシはステージングより高い可能性があり、計測していません。",
  "report": ".claude/report.html"
}
JSON
  tool Bash "adj gate open (調査報告)" "server: up · ボードで返事を待っています"
  say "調査報告をボードに出しました。詳しい内訳はレポートに付けています。"
  stop "調査報告をボードに出しました。詳しい内訳はレポートに付けています。"
}

worker_pr_checkout() {
  adj phase --set self-review > /dev/null
  tool Bash "git diff main --stat" "3 files changed, 24 insertions(+), 3 deletions(-)"
  python3 - "$DEMO_SRC/prs/57.diff" "$(task_of_worktree)" > "$DEMO_ROOT/diff-gate.json" <<'PY'
import json, sys
diff = open(sys.argv[1]).read()
print(json.dumps({
  "kind": "diff", "wait": False, "task": sys.argv[2],
  "title": "差分: 在庫の確認を 1 回の問い合わせにまとめる",
  "facts": ["セルフレビューは 2 周で収束", "verify 通過", "+24 / -3 行"],
  "diff": diff,
  "reviewRounds": [{"engine": "claude", "must": 1, "want": 0, "scope": 0, "falsePositives": 0},
                   {"engine": "claude", "must": 0, "want": 0, "scope": 0, "falsePositives": 0}],
  "findings": [{"severity": "must", "location": "src/checkout/checkout.ts:7", "text": "答えに無い SKU で undefined と比べていた", "outcome": "fixed"}],
}, ensure_ascii=False))
PY
  adj gate open --json < "$DEMO_ROOT/diff-gate.json" > "$DEMO_ROOT/gate-diff.out" 2>&1
  tool Bash "adj gate open (差分の記録)" "記録しました"
  adj phase --set pr > /dev/null
  tool Bash "git push -u origin demo/checkout-batch" "branch 'demo/checkout-batch' set up to track 'origin/demo/checkout-batch'"
  tool Bash "gh pr create --title 在庫の確認を 1 回の問い合わせにまとめる" "https://github.com/acme/shop-app/pull/57"
  say "PR #57 を作りました。レビューを待っています。"
  stop "PR #57 を作りました。レビューを待っています。"
}
