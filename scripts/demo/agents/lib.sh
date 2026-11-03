# Shared by the scripted agents: they draw a screen that reads like an agent's terminal and
# report to adjutant through the same hook a real Claude Code session runs.
SESSION_ID="${SESSION_ID:-}"
export CLAUDE_PID=$$

C_DIM=$'\e[2m'; C_OFF=$'\e[0m'; C_ACC=$'\e[38;5;173m'; C_OK=$'\e[32m'; C_B=$'\e[1m'

hook() { # hook EVENT [extra JSON members]
  local extra="${2:-}"
  printf '{"session_id":"%s","hook_event_name":"%s","cwd":"%s"%s}' \
    "$SESSION_ID" "$1" "$PWD" "${extra:+,$extra}" | adj hook claude >/dev/null 2>&1 || true
}

json_str() { python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"; }

banner() { # banner TITLE
  printf '%s✻%s %s%s%s\n' "$C_ACC" "$C_OFF" "$C_B" "$1" "$C_OFF"
  printf '  %s%s%s\n\n' "$C_DIM" "${WHERE:-~/src/shop-app}" "$C_OFF"
}

typeout() { # print a line a few characters at a time
  local s="$1" i
  for ((i = 0; i < ${#s}; i += 3)); do printf '%s' "${s:i:3}"; sleep 0.012; done
  printf '\n'
}

user() { printf '%s> %s%s\n\n' "$C_B" "$1" "$C_OFF"; hook UserPromptSubmit "\"prompt\":$(json_str "$1")"; }
say() { printf '%s⏺%s ' "$C_OK" "$C_OFF"; typeout "$1"; printf '\n'; }
tool() { # tool NAME ARG [RESULT]
  printf '%s⏺%s %s%s%s(%s)\n' "$C_OK" "$C_OFF" "$C_B" "$1" "$C_OFF" "$2"
  hook PostToolUse "\"tool_name\":\"$1\",\"tool_input\":{\"command\":$(json_str "$2"),\"file_path\":$(json_str "$2")}"
  [ -n "${3:-}" ] && printf '  %s⎿  %s%s\n' "$C_DIM" "$3" "$C_OFF"
  printf '\n'; sleep 0.4
}
# An input box drawn the way Claude Code draws one: a rule above and below, the text wrapped
# inside. A wake types only into an empty box, and presses Enter only once the box holds just
# the line it typed, so the box is redrawn as keys arrive rather than left to the tty's echo.
draw_box() { # draw_box TEXT
  local cols; cols=$(tput cols 2>/dev/null || echo 80)
  local rule; rule=$(printf '%*s' "$cols" '' | tr ' ' '─')
  local width=$((cols - 4)) text="$1" first=1
  printf '\e8\e[J%s%s%s\n' "$C_DIM" "$rule" "$C_OFF"
  while :; do
    if [ $first = 1 ]; then printf '❯ %s\n' "${text:0:width}"; first=0; else printf '  %s\n' "${text:0:width}"; fi
    text="${text:width}"; [ -z "$text" ] && break
  done
  printf '%s%s%s\n  %s⏵⏵ auto mode on%s' "$C_DIM" "$rule" "$C_OFF" "$C_DIM" "$C_OFF"
}
idle_prompt() { printf '\e7'; draw_box ""; }
read_typed() { # read_typed VAR: wait for a line typed into the box, then clear the box
  local line="" c more
  local saved; saved=$(stty -g)
  stty -echo -icanon min 1 time 0
  while IFS= read -r -n1 c; do
    if [ -z "$c" ]; then break; fi
    line+="$c"
    while IFS= read -r -n1 -t 0.03 more; do
      if [ -z "$more" ]; then draw_box "$line"; stty "$saved"; printf '\e8\e[J'; printf -v "$1" '%s' "$line"; return; fi
      line+="$more"
    done
    draw_box "$line"
  done
  stty "$saved"
  printf '\e8\e[J'
  printf -v "$1" '%s' "$line"
}
# The spinner line an agent shows mid-turn. It is wiped after a moment, as the agent wipes it when
# the turn moves on: left on screen it would make adjutant read the session as busy and hold a wake.
working() { printf '%s✻ %s… (esc to interrupt)%s' "$C_ACC" "$1" "$C_OFF"; sleep "${2:-1.2}"; printf '\r\e[2K'; }
stop() { hook Stop "\"last_assistant_message\":$(json_str "$1")"; }
