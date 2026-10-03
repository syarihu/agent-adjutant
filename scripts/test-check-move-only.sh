#!/usr/bin/env bash
# Pin scripts/check-move-only.sh against a fixture crate.
#
#   scripts/test-check-move-only.sh
#
# Moving a `mod tests { }` body into tests.rs de-indents it, and rustfmt then collapses a
# wrapped closure `|..| { expr }` to `|..| expr`. That must pass, while a changed token
# inside the closure, or dropped braces around a block that holds a `;`, must not. The head
# commits are written by hand so the result does not depend on the rustfmt version.
set -euo pipefail
export LC_ALL=C
unset CARGO_TARGET_DIR GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

script="$(cd "$(dirname "$0")" && pwd)/check-move-only.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/test-check-move-only.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
repo="$tmp/repo"
mkdir -p "$repo/src"
cd "$repo"

commit() {
  git add -A
  git -c user.name=t -c user.email=t@t -c commit.gpgsign=false -c core.hooksPath=/dev/null \
    commit -q -m "$1"
}

git init -q
git checkout -q -b main
printf '[package]\nname = "fixture"\nversion = "0.0.0"\nedition = "2024"\n' >Cargo.toml
printf 'target\n' >.gitignore
cat >src/lib.rs <<'RS'
pub fn one() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closures() {
        let check = |values: &[u8]| {
            values.iter().all(|value| *value == one() || *value == one() + 1)
        };
        let run = |values: &[u8]| {
            assert!(check(values));
        };
        let zeros = || {
            [one(); 2]
        };
        run(&[1, 2]);
        run(&zeros());
    }
}
RS
commit base

# Writes the moved head: <closure line of `check`> <closure line of `run`>.
write_head() {
  cat >src/lib.rs <<'RS'
pub fn one() -> u8 {
    1
}

#[cfg(test)]
mod tests;
RS
  cat >src/tests.rs <<RS
use super::*;

#[test]
fn closures() {
    let check = |values: &[u8]| $1;
    let run = |values: &[u8]| $2;
    let zeros = || [one(); 2];
    run(&[1, 2]);
    run(&zeros());
}
RS
}

failed=0
# expect <name> <exit code> <output pattern> <check body> <run body>
expect() {
  local name=$1 want=$2 pattern=$3 out code=0
  git checkout -q -B case main
  write_head "$4" "$5"
  commit "$name"
  out=$("$script" main 2>&1) || code=$?
  if [ "$code" = "$want" ] && grep -q "$pattern" <<<"$out"; then
    echo "ok: $name"
  else
    echo "FAIL: $name (exit $code, want $want)"
    echo "$out"
    failed=1
  fi
}

collapsed='values.iter().all(|value| *value == one() || *value == one() + 1)'
run_braced='{ assert!(check(values)); }'
expect "collapsed closure is a move" 0 "move-only ok" "$collapsed" "$run_braced"
expect "changed token inside the closure is not a move" 1 "not a move: the items above" \
  'values.iter().all(|value| *value == one() || *value == one() + 2)' "$run_braced"
expect "dropped braces around a block with a ; are not a move" 1 "not a move: the items above" \
  "$collapsed" 'assert!(check(values));'

# The other way: the base holds the collapsed closure and the head moves it deeper, back into
# an inline `mod tests`, where rustfmt wraps it in braces again.
git checkout -q -B reverse-base main
write_head "$collapsed" "$run_braced"
commit reverse-base
git checkout -q -B reverse-case reverse-base
git checkout -q main -- src/lib.rs
git rm -q src/tests.rs
commit "moved back into mod tests"
code=0
out=$("$script" reverse-base 2>&1) || code=$?
if [ "$code" = 0 ] && grep -q "move-only ok" <<<"$out"; then
  echo "ok: braces added by a deeper indent are a move"
else
  echo "FAIL: braces added by a deeper indent are a move (exit $code, want 0)"
  echo "$out"
  failed=1
fi

exit "$failed"
