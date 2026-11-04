#!/usr/bin/env bash
# Stop what start.sh started: the demo server and the demo tmux server.
source "$(dirname "$0")/env.sh"
adj server stop >/dev/null 2>&1 || true
dtmux kill-server 2>/dev/null || true
