#!/usr/bin/env bash
# Build a fresh demo world under $DEMO_ROOT: a fictional repository acme/shop-app, a config whose
# hub and worker are the scripted agents in agents/, a stub gh, and an isolated tmux server.
set -euo pipefail
source "$(dirname "$0")/env.sh"

dtmux kill-server 2>/dev/null || true
rm -rf "$DEMO_ROOT"
mkdir -p "$DEMO_ROOT"/{bin,state,gh-config,home/.claude,src}

cat > "$GIT_CONFIG_GLOBAL" <<'G'
[user]
	name = Demo
	email = demo@example.invalid
[init]
	defaultBranch = main
G

ln -s "$REPO_ROOT/target/debug/adj" "$DEMO_ROOT/bin/adj"
ln -s "$REPO_ROOT/target/debug/adjutant" "$DEMO_ROOT/bin/adjutant"
ln -s "$DEMO_SRC/gh-stub.py" "$DEMO_ROOT/bin/gh"

repo="$DEMO_ROOT/src/shop-app"
cp -R "$DEMO_SRC/shop-app" "$repo"
git -C "$repo" init -q
git -C "$repo" remote add origin git@github.com:acme/shop-app.git
git -C "$repo" add -A
git -C "$repo" commit -q -m "Initial import"

echo '{"mcpServers":{"adjutant":{"command":"adjutant","args":["mcp"]}}}' > "$CLAUDE_CONFIG_DIR/.claude.json"
sed -e "s|@AGENTS@|$DEMO_SRC/agents|g" "$DEMO_SRC/demo-config.json" > "$ADJUTANT_CONFIG"
printf 'set -g status off\n' > "$HOME/.tmux.conf"
echo "demo world ready in $DEMO_ROOT"
