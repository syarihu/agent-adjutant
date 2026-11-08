#!/usr/bin/env bash
# Start the demo server and the scripted hub in a world setup.sh built. Prints the board URL.
# Options are set after env.sh, so a refused source leaves the shell it was typed into as it was.
source "$(dirname "$0")/env.sh" || return 1 2>/dev/null || exit 1
set -euo pipefail
PORT="${DEMO_PORT:-4599}"
cd "$DEMO_ROOT/src/shop-app"
adj server start --foreground --no-open --port "$PORT" > "$DEMO_ROOT/server.log" 2>&1 &
for _ in $(seq 50); do [ -s "$ADJUTANT_STATE_DIR/dashboard-token" ] && break; sleep 0.1; done
adj hub --tab > /dev/null
echo "http://127.0.0.1:$PORT/?token=$(cat "$ADJUTANT_STATE_DIR/dashboard-token")"
