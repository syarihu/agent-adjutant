#!/usr/bin/env bash
# The investigation clip starts with a worker already looking into why checkout is slow.
source "$(dirname "$0")/common.sh"
DONE_WHEN=report-only start_worker checkout-slow "チェックアウトが遅い原因を調べる" investigate \
  "チェックアウトに数秒かかるという声がある。原因を調べて報告してほしい（直すかどうかは結果を見て決める）" > /dev/null
