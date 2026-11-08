#!/usr/bin/env bash
# The own-task review clip starts with a worker that has opened PR #57 for its task.
source "$(dirname "$0")/common.sh" || return 1 2>/dev/null || exit 1
task=$(start_worker checkout-batch "在庫の確認を 1 回の問い合わせにまとめる" start \
  "チェックアウトが遅い原因の調査結果を受けて、在庫 API を一括の問い合わせに変える")
adj task update --id "$task" --status pr --pr https://github.com/acme/shop-app/pull/57 > /dev/null
board_post /api/others/hub/start > /dev/null
