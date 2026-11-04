#!/usr/bin/env bash
# The scripted repository hub. A wake typed at its prompt makes it read the inbox the way a hub
# does (adj pending), and each message plays its part of a scene (scenes.sh).
NAME="$1"; SESSION_ID="$2"
source "$(dirname "$0")/lib.sh"
source "$(dirname "$0")/scenes.sh"
cd "$DEMO_ROOT/src/shop-app"
[ "${NAME#*review}" != "$NAME" ] && WHERE="~/.local/state/adjutant/review"
hook SessionStart
clear
banner "$([ "${NAME#*review}" != "$NAME" ] && echo "code-review hub" || echo "hub · acme/shop-app")"
say "hub を起動しました。受信箱を待っています。"
stop "hub を起動しました。受信箱を待っています。"
while :; do
  idle_prompt
  read_typed typed
  user "$typed"
  hook UserPromptSubmit
  working "受信箱を読んでいます"
  adj pending --json > "$DEMO_ROOT/pending.json"
  n=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["count"])' "$DEMO_ROOT/pending.json")
  tool Bash "adj pending" "$n 件"
  python3 -c 'import json,sys
for m in json.load(open(sys.argv[1]))["messages"]: print(m["name"]+"\t"+m["kind"]+"\t"+(m.get("subject") or ""))' "$DEMO_ROOT/pending.json" |
  while IFS=$'\t' read -r name kind subject; do
    body=$(adj pending --read "$name")
    printf '%s\n' "$body" > "$DEMO_ROOT/last-$kind.md"
    if hub_message "$name" "$kind" "$subject" "$body" < /dev/null; then
      adj pending --ack "$name" > /dev/null
    fi
  done
done
