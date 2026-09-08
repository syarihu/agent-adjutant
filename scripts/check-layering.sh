#!/usr/bin/env bash
# Keep the dependency arrows pointing one way.
#
# The Issue this grew out of settled on mod-per-concern inside one crate rather than a Cargo
# workspace: the shape here is a fan, not a stack, and splitting a fan into crates grows an
# empty relay layer between the two real ones. The cost of not splitting is that nothing
# machine-checks the direction — which is what this script buys back, for zero Cargo.toml.
#
# As long as this passes, splitting into crates later stays a mechanical move: files across,
# a Cargo.toml each, `crate::config::` -> `adjutant_config::`.
set -uo pipefail
cd "$(dirname "$0")/.."

# The bottom. Answers questions using nothing but the standard library and its own input.
BOTTOM=(config repo template prompts)
# The middle. May reach down, never sideways into a command and never up.
MIDDLE=(terminal runner notify ide messaging)

status=0

for name in "${BOTTOM[@]}"; do
  if hits=$(grep -n "^use crate::\|crate::" "src/$name.rs" | grep -v '^\s*//' | grep -v "crate::testing"); then
    echo "src/$name.rs is a leaf and must not depend on any other module:"
    echo "$hits"
    status=1
  fi
done

# `testing` is `#[cfg(test)]` scaffolding in lib.rs, not a layer: it ships in no binary and
# a leaf naming it says nothing about the direction of the arrows at run time.
allowed=$(IFS='|'; echo "${BOTTOM[*]}|testing")
for name in "${MIDDLE[@]}"; do
  if hits=$(grep -n "crate::" "src/$name.rs" | grep -Ev "crate::($allowed)::"); then
    echo "src/$name.rs may only depend on: ${BOTTOM[*]}"
    echo "$hits"
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "layering ok"
fi
exit "$status"
