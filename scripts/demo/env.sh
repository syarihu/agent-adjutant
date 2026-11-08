# Sourced by the demo scripts: everything adj, gh, git and tmux read points into $DEMO_ROOT,
# so a recording never touches the real state directory, config, GitHub login or tmux server.
# Only bash finds this file through BASH_SOURCE: sourced from another shell, the paths below would
# be wrong, so stop rather than let adj reach the real state.
if [ -z "${BASH_VERSION:-}" ]; then
  echo "scripts/demo: run the demo scripts with bash, not sourced from another shell" >&2
  return 1 2>/dev/null || exit 1
fi
DEMO_ROOT="${DEMO_ROOT:-/tmp/adj-demo}"
# A path that cannot be worked out stops here, before anything is reset or linked from it.
DEMO_SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" || return 1 2>/dev/null || exit 1
REPO_ROOT="$(cd "$DEMO_SRC/../.." && pwd)" || return 1 2>/dev/null || exit 1
export DEMO_ROOT DEMO_SRC REPO_ROOT
export ADJUTANT_CONFIG="$DEMO_ROOT/config.json"
export ADJUTANT_STATE_DIR="$DEMO_ROOT/state"
export GH_CONFIG_DIR="$DEMO_ROOT/gh-config"
export HOME="$DEMO_ROOT/home"
export CLAUDE_CONFIG_DIR="$DEMO_ROOT/home/.claude"
export CODEX_HOME="$DEMO_ROOT/home/.codex"
export GIT_CONFIG_GLOBAL="$DEMO_ROOT/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
unset TMUX TMUX_PANE GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GH_TOKEN GITHUB_TOKEN GH_HOST \
  ADJUTANT_HUB ADJUTANT_HUB_SESSION ADJUTANT_HUB_SERVE ADJUTANT_TMUX_SOCKET ADJUTANT_TMUX_SESSION \
  ADJUTANT_STARTUP_DASHBOARD XDG_STATE_HOME XDG_CONFIG_HOME
# The demo's own bin first: adj from this checkout, and a gh that answers from fixtures.
export PATH="$DEMO_ROOT/bin:$PATH"
DEMO_SOCKET=adj-demo
export DEMO_SOCKET
dtmux() { tmux -L "$DEMO_SOCKET" "$@"; }
