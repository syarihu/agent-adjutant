#!/usr/bin/env bash
# Check that a PR announced as a move only moves code.
#
#   scripts/check-move-only.sh <base>
#
# A move takes items from one file to another and changes only what the move forces: `mod`
# and `use` lines, visibility, and paths that point somewhere else from the new file. So
# both sides of every touched `src/**.rs` file are reduced to the tokens that are left once
# those are dropped, and the two multisets must be equal. Lines, indentation and wrapping do
# not count, so rustfmt re-wrapping a moved item is fine.
#
# It cannot see a reference retargeted to a same-named item elsewhere (`a::f` to `b::f`):
# the paths are dropped. The compiler and the test count at the end cover that.
#
# Known gaps: only the committed HEAD is checked, not the working tree; the closing brace of
# a `mod tests {` is found by its indent, so a differently indented one is missed.
#
# `git diff --color-moved` is not used: it does not mark blocks under 20 alphanumeric
# characters as moved, needs an option to see re-indented blocks, and does not check that
# every removed line came back.
set -euo pipefail
export LC_ALL=C

if [ $# -ne 1 ]; then
  echo "usage: $0 <base>" >&2
  exit 2
fi
cd "$(git rev-parse --show-toplevel)"
if [ -n "$(git status --porcelain -- src)" ]; then
  echo "warning: uncommitted changes under src/ are not checked; only HEAD is" >&2
fi

# Compare with where the branch left <base>, not with <base> itself: a <base> that moved on
# since would bring its own changes into the comparison.
base=$(git merge-base "$1" HEAD)

files=$(git diff --name-only --no-renames "$base" HEAD -- src | grep '\.rs$' || true)
if [ -z "$files" ]; then
  echo "move-only ok: no .rs file under src/ changed"
  exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/check-move-only.XXXXXX")
cleanup() {
  if [ -d "$tmp/base" ]; then
    git worktree remove --force "$tmp/base" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp"
  git worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# One record per file and side: `side<TAB>path<NUL>contents<NUL>`.
for f in $files; do
  for side in base head; do
    if [ "$side" = base ]; then rev=$base; else rev=HEAD; fi
    printf '%s\t%s\0' "$side" "$f"
    git show "$rev:$f" 2>/dev/null || true
    printf '\0'
  done
done >"$tmp/records"

# --- 1-4: the tokens ---------------------------------------------------------------------
if ! perl -0 -e '
  use strict;
  use warnings;

  # `a/b/../c` -> `a/c`, relative to the repository root.
  sub resolve {
    my ($file, $lit) = @_;
    my @parts = split m{/}, $file;
    pop @parts;
    for my $p (split m{/}, $lit) {
      next if $p eq "" || $p eq ".";
      if ($p eq "..") { pop @parts } else { push @parts, $p }
    }
    return join "/", @parts;
  }

  sub normalize {
    my ($file, $src) = @_;
    my @out;
    my $in_use = 0;
    my $tests_indent;    # set while inside `mod tests {`: the indent of its closing brace
    for my $line (split /\n/, $src, -1) {
      if ($in_use) { $in_use = 0 if $line =~ /;\s*$/; next; }
      if (defined $tests_indent && $line =~ /^\Q$tests_indent\E\}\s*$/) {
        undef $tests_indent;
        next;
      }
      next if $line =~ m{^\s*//!};
      next if $line =~ /^\s*#\[cfg\(test\)\]\s*$/;
      next if $line =~ /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;/;
      if ($line =~ /^(\s*)mod\s+tests\s*\{\s*$/) { $tests_indent = $1; next; }
      if ($line =~ /^\s*(?:pub(?:\([^)]*\))?\s+)?use\s/) {
        $in_use = 1 unless $line =~ /;\s*$/;
        next;
      }
      push @out, $line;
    }
    my $text = join "\n", @out;
    $text =~ s{\binclude_(str|bytes)!\(\s*"([^"]*)"\s*(,\s*)?\)}
              {"include_$1!(\"" . resolve($file, $2) . "\")"}ge;
    $text =~ s/\bpub\s*\(\s*(?:crate|super|self|in\s+[^)]*)\s*\)//g;
    $text =~ s/\bpub\b//g;
    $text =~ s/(?:(?:\$crate|\b[A-Za-z_]\w*)\s*::\s*)+//g;
    $text =~ s/,//g;
    return $text =~ /[A-Za-z0-9_]+|\S/g;
  }

  my %count;    # token -> base minus head
  my %where;    # token -> side -> { file -> n }
  local $/ = "\0";
  while (defined(my $head = <STDIN>)) {
    chomp $head;
    my $src = <STDIN>;
    $src = "" unless defined $src;
    chomp $src;
    my ($side, $file) = split /\t/, $head, 2;
    for my $t (normalize($file, $src)) {
      $count{$t} += $side eq "base" ? 1 : -1;
      $where{$t}{$side}{$file}++;
    }
  }

  my $bad = 0;
  for my $t (sort keys %count) {
    my $n = $count{$t};
    next if $n == 0;
    my $side = $n > 0 ? "base" : "head";
    my $files = join ", ", map { "$_ ($where{$t}{$side}{$_})" } sort keys %{ $where{$t}{$side} };
    printf "%s: %d more `%s` than %s, in %s\n",
      $side eq "base" ? "removed" : "added", abs $n, $t,
      $side eq "base" ? "added" : "removed", $files;
    $bad = 1;
  }
  exit $bad;
' <"$tmp/records"; then
  echo "not a move: the tokens above are on one side only" >&2
  exit 1
fi

# --- 5: the test count ---------------------------------------------------------------------
# The base is built in a worktree with a target directory of its own. Sharing HEAD's does not
# work: cargo names the package's artifacts by a hash that leaves out its path, so the two
# checkouts write the same files and the second build, whose sources are older than the
# first one's output, is taken as fresh and skipped. The base's directory sits inside HEAD's
# so its dependencies stay cached between runs.
target=${CARGO_TARGET_DIR:-$PWD/target}
count_tests() {
  if ! (cd "$1" && CARGO_TARGET_DIR=$2 cargo test --quiet -- --list) >"$tmp/list" 2>"$tmp/err"; then
    cat "$tmp/err" >&2
    echo "cargo test -- --list failed in $1" >&2
    exit 1
  fi
  grep -c ': test$' "$tmp/list" || true
}
git worktree add --quiet --detach "$tmp/base" "$base"
base_tests=$(count_tests "$tmp/base" "$target/check-move-only-base")
head_tests=$(count_tests "$PWD" "$target")
if [ "$base_tests" != "$head_tests" ]; then
  echo "not a move: $base_tests tests at the base, $head_tests at HEAD" >&2
  exit 1
fi
echo "move-only ok: same tokens in $(echo "$files" | wc -l | tr -d ' ') files, $head_tests tests on both sides"
