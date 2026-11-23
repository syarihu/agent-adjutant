#!/usr/bin/env bash
# The macOS app ships in the same release tarball as adjutant and adj, so its version has to be the
# root package's. The app crate is its own workspace and cannot inherit it, and tauri.conf.json has
# no version of its own so that the bundle takes the app crate's.
set -euo pipefail
cd "$(dirname "$0")/.."

package_version() {
  sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$1" | head -n 1
}

root=$(package_version Cargo.toml)
app=$(package_version app/src-tauri/Cargo.toml)
status=0

if [ -z "$root" ] || [ "$root" != "$app" ]; then
  echo "app/src-tauri/Cargo.toml has version \"$app\", but Cargo.toml has \"$root\": bump both together." >&2
  status=1
fi

if grep -q '"version"' app/src-tauri/tauri.conf.json; then
  echo "app/src-tauri/tauri.conf.json sets a version: remove it so the app takes app/src-tauri/Cargo.toml's." >&2
  status=1
fi

# CI bundles the app on every PR to catch what would break the release, so both use one Tauri CLI.
ci_cli=$(sed -n 's/^ *TAURI_CLI_VERSION: "\(.*\)"/\1/p' .github/workflows/ci.yml)
release_cli=$(sed -n 's/^ *TAURI_CLI_VERSION: "\(.*\)"/\1/p' .github/workflows/release.yml)
if [ -z "$ci_cli" ] || [ "$ci_cli" != "$release_cli" ]; then
  echo "TAURI_CLI_VERSION is \"$ci_cli\" in ci.yml but \"$release_cli\" in release.yml: set both to one version." >&2
  status=1
fi

exit "$status"
