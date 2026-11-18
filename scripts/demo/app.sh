#!/usr/bin/env bash
# app.sh [Adjutant.app]: build a fresh demo world, seed the board, and open the macOS app on it,
# for screenshots. Nothing of the real state, config, GitHub login or tmux server is read.
#
# The app must be built with an identifier of its own, so that it neither hands itself over to an
# Adjutant that is already running (the single-instance socket is named after the identifier) nor
# reads the real app's settings:
#   (cd app/src-tauri && cargo tauri build --bundles app --config '{"identifier":"dev.syarihu.adjutant.demo"}')
# Stop it with scripts/demo/stop.sh after quitting the app.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="${1:-$here/../../app/src-tauri/target/release/bundle/macos/Adjutant.app}"
bin="$app/Contents/MacOS/adjutant-app"
id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")
if [ "$id" = dev.syarihu.adjutant ]; then
  echo "app.sh: $app has the real app's identifier; build it with another one (see the top of this file)" >&2
  exit 1
fi

bash "$here/stop.sh"
bash "$here/setup.sh" > /dev/null
bash "$here/start.sh" > /dev/null
sleep 2
bash "$here/scenes/boards-list.sh"
bash "$here/scenes/review-others.sh" > /dev/null

source "$here/env.sh"
# HOME is the demo's, so the app's settings live in the demo world too.
conf="$HOME/Library/Application Support/$id"
mkdir -p "$conf"
cat > "$conf/settings.json" <<J
{ "adjPath": "$DEMO_ROOT/bin/adj", "background": "material", "shortcut": null }
J
nohup "$bin" > "$DEMO_ROOT/app.log" 2>&1 &
echo "app started (pid $!); quit it, then run scripts/demo/stop.sh"
