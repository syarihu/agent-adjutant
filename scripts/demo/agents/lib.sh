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
# BOX_ON says the box is on screen and BOX_TEXT what it holds, so that a resize can draw it again.
BOX_ON=0; BOX_TEXT=""
draw_box() { # draw_box TEXT
  local cols; cols=$(term_cols)
  local rule="" i; for ((i = 0; i < cols; i++)); do rule+="─"; done
  local width=$((cols > 5 ? cols - 4 : 1)) text="$1" first=1
  BOX_ON=1; BOX_TEXT="$1"
  printf '\e8\e[J%s%s%s\n' "$C_DIM" "$rule" "$C_OFF"
  while :; do
    if [ $first = 1 ]; then printf '❯ %s\n' "${text:0:width}"; first=0; else printf '  %s\n' "${text:0:width}"; fi
    text="${text:width}"; [ -z "$text" ] && break
  done
  printf '%s%s%s\n  %s⏵⏵ auto mode on%s' "$C_DIM" "$rule" "$C_OFF" "$C_DIM" "$C_OFF"
}
# The width is read from the tty itself: `tput cols` in a command substitution writes to a pipe,
# and does not always find the tty's size from there.
term_cols() {
  local size; size=$(stty size < /dev/tty 2>/dev/null); size="${size#* }"
  if [[ "$size" =~ ^[1-9][0-9]*$ ]]; then echo "$size"; else echo 80; fi
}
# Claude Code draws its box again at the new width when the terminal is resized. The board's
# terminal attaches to the pane at its own size, after the box was drawn at the pane's first
# width: without this the rules would stop where that width ended. tmux has reflowed the screen
# by then (a narrower pane wraps the rules onto more rows, and a wrapped line is not always joined
# again when it gets wider), so the box's first row is found on the screen tmux holds: up from the
# cursor, on the box's last line, past the rows of the lower rule, the text and the upper rule.
redraw_box() {
  [ "$BOX_ON" = 1 ] && [ -n "${TMUX_PANE:-}" ] || return 0
  local cy rows y state=0 line
  cy=$(tmux display -p -t "$TMUX_PANE" '#{cursor_y}' 2>/dev/null) || return 0
  rows=()
  while IFS= read -r line; do rows+=("$line"); done < <(tmux capture-pane -p -t "$TMUX_PANE" 2>/dev/null)
  # With no screen to read, nothing says where the box starts: leave it as it is.
  [ "${#rows[@]}" -gt "$cy" ] || return 0
  for ((y = cy - 1; y >= 0; y--)); do
    line="${rows[y]:-}"
    if [[ "$line" =~ ^─+$ ]]; then
      [ $state = 1 ] && state=2
    elif [ $state = 0 ]; then state=1
    elif [ $state = 2 ]; then break
    fi
  done
  printf '\r'
  [ $((cy - y - 1)) -gt 0 ] && printf '\e[%dA' $((cy - y - 1))
  printf '\e7'
  draw_box "$BOX_TEXT"
}
trap redraw_box WINCH
idle_prompt() { printf '\e7'; draw_box ""; }
read_typed() { # read_typed VAR: wait for a line typed into the box, then clear the box
  local line="" c more status poll=()
  local saved; saved=$(stty -g)
  stty -echo -icanon min 1 time 0
  # bash runs a trap only once `read` returns, so the wait for the first key is cut into short
  # reads, and a resize is drawn while nothing is typed. bash 3.2 takes no fraction of a second,
  # so there the wait stays one read (3.2 runs the trap during it).
  ((BASH_VERSINFO[0] >= 4)) && poll=(-t 0.2)
  while :; do
    IFS= read -r -n1 "${poll[@]}" c; status=$?
    [ "$status" -gt 128 ] && continue
    [ "$status" = 0 ] && [ -n "$c" ] || break
    line+="$c"
    while IFS= read -r -n1 -t 0.03 more; do
      if [ -z "$more" ]; then draw_box "$line"; BOX_ON=0; stty "$saved"; printf '\e8\e[J'; printf -v "$1" '%s' "$line"; return; fi
      line+="$more"
    done
    draw_box "$line"
  done
  BOX_ON=0
  stty "$saved"
  printf '\e8\e[J'
  printf -v "$1" '%s' "$line"
}
# The spinner line an agent shows mid-turn. It is wiped after a moment, as the agent wipes it when
# the turn moves on: left on screen it would make adjutant read the session as busy and hold a wake.
working() { printf '%s✻ %s… (esc to interrupt)%s' "$C_ACC" "$1" "$C_OFF"; sleep "${2:-1.2}"; printf '\r\e[2K'; }
stop() { hook Stop "\"last_assistant_message\":$(json_str "$1")"; }
