#!/usr/bin/env bash
# Record every clip in both languages.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
for clip in request-plan investigate-result review-own review-others; do
  for lang in ja en; do
    echo "== $clip $lang"
    bash "$here/record.sh" "$clip" "$lang"
  done
done
