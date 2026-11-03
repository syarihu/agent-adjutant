#!/usr/bin/env bash
# record.sh <clip> <ja|en>: build a fresh demo world, play the clip in a browser, and write
# docs/images/board-<clip>[.ja].mp4 and .gif.
set -euo pipefail
clip="$1"; lang="$2"
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../../docs/images"
work="${DEMO_OUT:-/tmp/adj-demo-frames}"
bash "$here/stop.sh"
bash "$here/setup.sh" > /dev/null
url=$(bash "$here/start.sh")
sleep 2
if [ -f "$here/scenes/$clip.sh" ]; then bash "$here/scenes/$clip.sh"; sleep 3; fi
node "$here/record/run.mjs" "$clip" "$lang" "$url" "$work/$clip-$lang"
bash "$here/stop.sh"
suffix=$([ "$lang" = ja ] && echo .ja || true)
mkdir -p "$out"
node "$here/record/captions.mjs" "$work/$clip-$lang"
( cd "$work/$clip-$lang" &&
  ffmpeg -v error -y -f concat -safe 0 -i frames.txt -f concat -safe 0 -i captions.txt \
    -filter_complex "[0:v]fps=30,scale=2560:1520[v];[1:v]fps=30,scale=2560:128[c];[v][c]vstack=shortest=1,scale=1600:-2:flags=lanczos,format=yuv420p" \
    -c:v libx264 -crf 22 -preset slow -movflags +faststart "$out/board-$clip$suffix.mp4" &&
  ffmpeg -v error -y -i "$out/board-$clip$suffix.mp4" -vf "fps=12,scale=1000:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    "$out/board-$clip$suffix.gif" )
ls -la "$out/board-$clip$suffix".*
