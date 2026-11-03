#!/usr/bin/env bash
# The scripted reader (AI 下読み): it collects the material with adj review context, reads for a
# while, and hands in the read-through prepared for that PR in prs/.
NAME="$1"; SESSION_ID="$2"; shift 2
source "$(dirname "$0")/lib.sh"
id=$(printf '%s\n' "$*" | grep -oE '[a-z-]+/[a-z-]+#[0-9]+' | head -1)
number="${id##*#}"
WHERE="AI 下読み · $id"
hook SessionStart
clear
banner "reader · $id"
user "Read $id and write the read-through."
adj review context --pr "$id" --out . > context.out 2>&1
tool Bash "adj review context --pr $id --out ." "pr.json, diff.patch, review-comments.json"
working "差分を読んでいます" 2
tool Read "diff.patch" "$(wc -l < diff.patch | tr -d ' ') lines"
working "解説と気になるところを書いています" 2.5
sha=$(python3 -c 'import json;print(json.load(open("pr.json"))["headRefOid"])')
sed "s/@SHA@/$sha/" "$DEMO_SRC/prs/$number.read.json" > read-through.json
adj review submit --pr "$id" --file read-through.json > submit.out 2>&1
tool Bash "adj review submit --pr $id --file read-through.json" "$(head -1 submit.out | cut -c1-70)"
say "下読みを出しました。差分のことはこのウィンドウで聞いてください。"
stop "下読みを出しました。差分のことはこのウィンドウで聞いてください。"
while :; do idle_prompt; read_typed typed; user "$typed"; say "差分を見て答えます。"; stop "差分を見て答えます。"; done
