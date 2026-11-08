#!/usr/bin/env bash
# The search palette clip uses the same seeded board as the list clip.
source "$(dirname "$0")/boards-list.sh" || return 1 2>/dev/null || exit 1
