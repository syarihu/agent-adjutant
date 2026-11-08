#!/usr/bin/env bash
# The review clip starts with the code-review hub running and the review requests synced.
source "$(dirname "$0")/common.sh" || return 1 2>/dev/null || exit 1
board_post /api/others/hub/start > /dev/null
sleep 2
board_post /api/others/sync > /dev/null
