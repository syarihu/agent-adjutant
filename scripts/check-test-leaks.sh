#!/bin/sh
# Run a command under a private TMPDIR and fail if it leaves processes behind.
#
#   scripts/check-test-leaks.sh cargo test
#
# Every fixture, tmux server and agent a test starts has its working directory under TMPDIR,
# so a process whose cwd is still under the private one after the command is a leak. A login
# shell left in a dead tmux server holds a pty for good, and the machine runs out of them
# (`fork failed: Device not configured`) after a day of test runs.
dir=$(mktemp -d /tmp/adj-check.XXXXXX) && dir=$(cd "$dir" && pwd -P) || exit 1
# In the background so that a signal is handled at once rather than after the command, and
# still followed by the check: a test killed before its Drop is exactly what leaks. A
# background child ignores INT, so it is passed on as TERM.
caught=
trap 'caught=1; kill -TERM "$pid" 2>/dev/null' INT TERM
TMPDIR="$dir/" "$@" &
pid=$!
wait "$pid"
status=$?
while [ -n "$caught" ] && kill -0 "$pid" 2>/dev/null; do
  wait "$pid"
done
[ -n "$caught" ] && status=130
if ! command -v lsof >/dev/null 2>&1; then
  echo "check-test-leaks: lsof not found, leak check skipped" >&2
  rm -rf "$dir"
  exit $status
fi
# A process killed on the way out may take a moment to be gone.
sleep 2
left=$(lsof -a -d cwd -u "$(id -u)" -Fpn 2>/dev/null |
  awk -v d="$dir" '/^p/{p=substr($0,2)} /^n/{n=substr($0,2); if (n==d || index(n, d "/")==1) print p}' | sort -u)
if [ -n "$left" ]; then
  echo "the test run left processes behind (cwd under $dir):" >&2
  ps -o pid=,ppid=,lstart=,args= -p "$(echo $left | tr ' ' ',')" >&2
  [ "$status" -eq 0 ] && status=1
fi
rm -rf "$dir"
exit $status
