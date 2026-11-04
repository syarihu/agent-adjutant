#!/usr/bin/env bash
# Start the demo server and the scripted hub in a world setup.sh built. Prints the board URL.
set -euo pipefail
source "$(dirname "$0")/env.sh"
PORT="${DEMO_PORT:-4599}"
cd "$DEMO_ROOT/src/shop-app"
adj server start --foreground --no-open --port "$PORT" > "$DEMO_ROOT/server.log" 2>&1 &
for _ in $(seq 50); do [ -s "$ADJUTANT_STATE_DIR/dashboard-token" ] && break; sleep 0.1; done
adj hub --tab > /dev/null
echo "http://127.0.0.1:$PORT/?token=$(cat "$ADJUTANT_STATE_DIR/dashboard-token")"
