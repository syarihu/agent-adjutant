#!/usr/bin/env bash
# Pin scripts/check-move-only.sh against a fixture crate.
#
#   scripts/test-check-move-only.sh
#
# Moving a `mod tests { }` body into tests.rs de-indents it, and rustfmt then collapses a
# wrapped closure `|..| { expr }` to `|..| expr`. That must pass, while a changed token
# inside the closure, or dropped braces around a block that holds a `;`, must not. The head
# commits are written by hand so the result does not depend on the rustfmt version.
#
# A `mod x;` or `use` line takes the attributes and comments above it along, so moving
# `#[cfg(unix)] mod x;`, or adding the attribute where it lands, must pass, while changing an
# attribute on a moved function must not.
set -euo pipefail
export LC_ALL=C
unset CARGO_TARGET_DIR GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

script="$(cd "$(dirname "$0")" && pwd)/check-move-only.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/test-check-move-only.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# new_crate <dir>: an empty crate on `main`, entered.
new_crate() {
  mkdir -p "$1/src"
  cd "$1"
  git init -q
  git checkout -q -b main
  printf '[package]\nname = "fixture"\nversion = "0.0.0"\nedition = "2024"\n' >Cargo.toml
  printf 'target\n' >.gitignore
}

commit() {
  git add -A
  git -c user.name=t -c user.email=t@t -c commit.gpgsign=false -c core.hooksPath=/dev/null \
    commit -q -m "$1"
}

new_crate "$tmp/repo"
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
# verify <name> <exit code> <output pattern> <base>: commits the case and checks it against <base>.
verify() {
  local name=$1 want=$2 pattern=$3 out code=0
  commit "$name"
  out=$("$script" "$4" 2>&1) || code=$?
  if [ "$code" = "$want" ] && grep -q "$pattern" <<<"$out"; then
    echo "ok: $name"
  else
    echo "FAIL: $name (exit $code, want $want)"
    echo "$out"
    failed=1
  fi
}
# expect <name> <exit code> <output pattern> <check body> <run body>
expect() {
  git checkout -q -B case main
  write_head "$4" "$5"
  verify "$1" "$2" "$3" main
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
verify "braces added by a deeper indent are a move" 0 "move-only ok" reverse-base

# Attributes and comments above a dropped `mod x;` or `use` line go with it.
new_crate "$tmp/attrs"

# write_lib <declarations under `mod infra;`> <items above the tests>
write_lib() {
  cat >src/lib.rs <<RS
#![allow(dead_code)]

mod infra;
$1

$2

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles() {
        assert_eq!(double(helper()), 2);
    }
}
RS
}
# write_infra <declarations> <items below triple>
write_infra() {
  cat >src/infra.rs <<RS
$1

pub fn triple(x: u8) -> u8 {
    x * 3
}

$2
RS
}

pty_mod=$'#[cfg(unix)]\nmod pty;'
double=$'/// Doubles a number.\n#[inline]\npub fn double(x: u8) -> u8 {\n    x * 2\n}'
helper=$'#[cfg(test)]\nfn helper() -> u8 {\n    1\n}'

write_lib "$pty_mod"$'\nmod shell;' "$helper"$'\n\n'"$double"
write_infra "pub mod clock;" ""
mkdir -p src/infra
for m in pty:open shell:run infra/clock:now; do
  printf 'pub fn %s() -> u8 {\n    0\n}\n' "${m#*:}" >"src/${m%:*}.rs"
done
commit base

attrs_case() {
  git checkout -q -B case main
}

attrs_case
write_lib $'mod shell;' "$helper"$'\n\n'"$double"
write_infra $'#[cfg(unix)]\npub mod pty;\npub mod clock;' ""
git mv src/pty.rs src/infra/pty.rs
verify "moving #[cfg(unix)] mod x; to another file is a move" 0 "move-only ok" main

attrs_case
write_lib "$pty_mod" "$helper"$'\n\n'"$double"
write_infra $'pub mod clock;\n#[cfg(unix)]\npub mod shell;' ""
git mv src/shell.rs src/infra/shell.rs
verify "adding #[cfg(unix)] only where a mod lands is a move" 0 "move-only ok" main

attrs_case
write_lib "$pty_mod" "$helper"$'\n\n'"$double"
write_infra $'pub mod clock;\n#[cfg(any(\n    unix,\n    windows\n))]\npub mod shell;' ""
git mv src/shell.rs src/infra/shell.rs
verify "a wrapped attribute goes with its mod" 0 "move-only ok" main

attrs_case
write_infra $'/// The clock.\n#[cfg(unix)]\npub mod clock;' ""
verify "a doc and #[cfg(unix)] added to a mod in place are a move" 0 "move-only ok" main

attrs_case
write_lib "$pty_mod"$'\nmod shell;\npub use infra::double;' "$helper"
write_infra "pub mod clock;" "$double"
verify "a moved function with its attributes is a move" 0 "move-only ok" main

attrs_case
write_lib "$pty_mod"$'\nmod shell;\npub use infra::double;' "$helper"
write_infra "pub mod clock;" "${double/\#\[inline\]/#[inline(always)]}"
verify "a changed attribute on a moved function is not a move" 1 "not a move: the items above" main

attrs_case
write_lib "$pty_mod"$'\nmod shell;\n#[cfg(unix)]\npub use infra::triple;' "$helper"$'\n\n'"$double"
verify "#[cfg(unix)] above a use goes with it" 0 "move-only ok" main

# A plain `//` comment above a mod goes with it. The base carries it, so it has a base of its own.
git checkout -q -B comment-base main
write_lib $'// Terminal plumbing.\n'"$pty_mod"$'\nmod shell;' "$helper"$'\n\n'"$double"
commit comment-base
git checkout -q -B comment-case comment-base
write_lib $'mod shell;' "$helper"$'\n\n'"$double"
write_infra $'// Terminal plumbing.\n#[cfg(unix)]\npub mod pty;\npub mod clock;' ""
git mv src/pty.rs src/infra/pty.rs
verify "a // comment above a moved mod goes with it" 0 "move-only ok" comment-base

# An inner attribute right above a dropped mod is not held back: a change to it still shows.
git checkout -q -B inner-base main
sed -i.bak '2d' src/lib.rs
rm src/lib.rs.bak
commit inner-base
git checkout -q -B inner-case inner-base
sed -i.bak 's/dead_code/unused/' src/lib.rs
rm src/lib.rs.bak
verify "a changed inner attribute above a mod is not a move" 1 "not a move: the items above" inner-base

# An attribute that closes on a line with code after it: the code is compared, and a mod
# line below it does not take it away. The base has the one-line function above a mod.
git checkout -q -B closing-base main
write_lib $'#[inline(\n    always\n)] pub fn quad(x: u8) -> u8 { x * 4 }\nmod shell;' "$helper"$'\n\n'"$double"
commit closing-base
git checkout -q -B closing-case closing-base
sed -i.bak 's/x \* 4/x * 5/' src/lib.rs
rm src/lib.rs.bak
verify "a changed body after a closing attribute is not a move" 1 "not a move: the items above" closing-base

# A `//` inside a string in an attribute is not a comment.
git checkout -q -B case main
write_lib "$pty_mod" "$helper"$'\n\n'"$double"
write_infra $'pub mod clock;\n#[doc = "https://example.invalid"] pub mod shell;' ""
git mv src/shell.rs src/infra/shell.rs
verify "a // inside a string in an attribute is not a comment" 0 "move-only ok" main

# A string in an attribute can hold `]` and run over several lines: it is read as text.
git checkout -q -B string-base main
write_lib $'mod shell;' "$helper"$'\n\n#[doc = "old text\n] mod pty;\n"]\nfn target() {}\n\n'"$double"
commit string-base
git checkout -q -B string-case string-base
sed -i.bak 's/old text/new text/' src/lib.rs
rm src/lib.rs.bak
verify "a changed doc string with a ] mod line inside is not a move" 1 "not a move: the items above" string-base

git checkout -q -B case main
write_lib "$pty_mod" "$helper"$'\n\n'"$double"
write_infra $'pub mod clock;\n#[doc = "a]b"] pub mod shell;' ""
git mv src/shell.rs src/infra/shell.rs
verify "a ] inside a string in an attribute does not end it" 0 "move-only ok" main

# A helper moved from a test file into tests/common/mod.rs is read like one moved under src/.
new_crate "$tmp/tests"
mkdir -p tests
printf '' >src/lib.rs
# write_test_head <factor in the moved helper>
write_test_head() {
  mkdir -p tests/common
  cat >tests/a.rs <<'RS'
mod common;

use common::*;

#[test]
fn doubles() {
    assert_eq!(double(2), 4);
}
RS
  cat >tests/common/mod.rs <<RS
//! What the tests share.

pub fn double(x: u8) -> u8 {
    x * $1
}
RS
}
cat >tests/a.rs <<'RS'
fn double(x: u8) -> u8 {
    x * 2
}

#[test]
fn doubles() {
    assert_eq!(double(2), 4);
}
RS
commit base
git checkout -q -B case main
write_test_head 2
verify "a helper moved into tests/common/mod.rs is a move" 0 "same items in 2 files" main
git checkout -q -B case main
write_test_head 3
verify "a helper changed on its way into tests/common/mod.rs is not a move" 1 "not a move: the items above" main

exit "$failed"
