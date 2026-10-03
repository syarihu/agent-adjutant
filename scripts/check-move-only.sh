#!/usr/bin/env bash
# Check that a PR announced as a move only moves code.
#
#   scripts/check-move-only.sh <base>
#
# A move takes items from one file to another and changes only what the move forces: `mod`
# and `use` lines, visibility, and paths that point somewhere else from the new file. So
# both sides of every touched `src/**.rs` file are reduced to the tokens that are left once
# those are dropped, cut into top-level items (up to the `}` that closes one, or a `;`), and
# the two multisets of items must be equal. Each item keeps its tokens in order, so moving a
# whole item anywhere is fine, while changing or reordering tokens within one is not. Lines,
# indentation and wrapping do not count, so rustfmt re-wrapping a moved item is fine.
#
# A `mod x;` or `use` line takes the outer attributes and the `///` and `//` comments directly
# above it along (up to a blank line or code), so a declaration moves with its `#[cfg(unix)]`.
#
# A closure whose body is a block with no `;` of its own (`|x| { e }`) is read without the
# braces on both sides: rustfmt drops them when a shallower indent lets `e` fit, and adds
# them back when a deeper one does not. A `;` in the block, or an `-> T` before it, keeps
# them.
#
# It cannot see a reference retargeted to a same-named item elsewhere (`a::f` to `b::f`):
# the paths are dropped. The compiler and the test count at the end cover that.
#
# Known gaps: only the committed HEAD is read, so a dirty src/ is refused; the closing brace
# of a `mod tests {` is found by its indent, so a differently indented one is missed. Moving
# items out of a non-test inline `mod x { }` or out of an `impl` block shows as a change: a
# false failure, never a false pass. Braces inside string or char literals can do the same.
# `a || { b }` loses its braces too, so adding or removing just those braces passes, and so
# does `|| { e }` inside a macro that reads its tokens as text (`stringify!`): closures in
# macro arguments such as `assert!` are the ones rustfmt collapses, so macros are not skipped.
# An attribute on a `mod x;` or `use` line is not compared, so adding, changing or dropping
# its `#[cfg]` or `#[path]` passes; the compiler and the test count are left to see it. A
# comment left behind above a moved `mod x;` or `use` line shows as a change. An attribute is
# followed to its `]` by counting brackets outside `"..."`, so a bracket in a raw string or
# char literal can hold the lines up to the next blank one, and a `mod` or `use` line that
# ends them drops them.
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
  echo "commit or stash changes under src/ first: the tokens are read from HEAD and the tests are counted in the working tree" >&2
  exit 1
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
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

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

  # How far a line opens `[`, leaving out string literals and a trailing comment.
  sub brackets {
    (my $s = shift) =~ s/"(?:[^"\\]|\\.)*"//g;
    $s =~ s{//.*$}{};
    return ($s =~ tr/[//) - ($s =~ tr/]//);
  }

  sub normalize {
    my ($file, $src) = @_;
    my @out;
    my @held;      # attributes and comments directly above the next line of code
    my $attr = 0;  # bracket depth while an attribute runs over several lines
    my $in_use = 0;
    my $tests_indent;    # set while inside `mod tests {`: the indent of its closing brace
    for my $line (split /\n/, $src, -1) {
      # A trailing `// ...` must not hide the `;` or brace that ends the line. Only the
      # tests on `$code` look at it; the line itself is kept whole.
      (my $code = $line) =~ s{\s*//.*$}{};
      if ($in_use) { $in_use = 0 if $code =~ /;\s*$/; next; }
      if (defined $tests_indent && $code =~ /^\Q$tests_indent\E\}\s*$/) {
        undef $tests_indent;
        next;
      }
      next if $line =~ m{^\s*//!};
      next if $code =~ /^\s*#\[cfg\(test\)\]\s*$/;
      if ($attr > 0) {
        # rustfmt never puts a blank line inside an attribute: the count went wrong, keep all.
        if ($line !~ /\S/) { push @out, @held, $line; @held = (); $attr = 0; next; }
        push @held, $line;
        $attr += brackets($line);
        next;
      }
      if ($code =~ /^\s*#\[/) {    # an outer attribute; `#![` does not match
        my $depth = brackets($line);
        if ($depth > 0) { push @held, $line; $attr = $depth; next; }
        $code =~ s/^(\s*)(?:#(\[(?:[^\[\]]++|(?-1))*\])\s*)+/$1/;
        if ($code !~ /\S/) { push @held, $line; next; }
      }
      if ($line =~ m{^\s*//}) { push @held, $line; next; }    # `///` or `//`; `//!` is gone above
      if ($line !~ /\S/) { push @out, @held, $line; @held = (); next; }
      if ($code =~ /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;/) { @held = (); next; }
      if ($code =~ /^(\s*)mod\s+tests\s*\{\s*$/) { @held = (); $tests_indent = $1; next; }
      if ($code =~ /^\s*(?:pub(?:\([^)]*\))?\s+)?use\s/) {
        @held = ();
        $in_use = 1 unless $code =~ /;\s*$/;
        next;
      }
      push @out, @held, $line;
      @held = ();
    }
    push @out, @held;
    my $text = join "\n", @out;
    $text =~ s{\binclude_(str|bytes)!\(\s*"([^"]*)"\s*(,\s*)?\)}
              {"include_$1!(\"" . resolve($file, $2) . "\")"}ge;
    $text =~ s/\bpub\s*\(\s*(?:crate|super|self|in\s+[^)]*)\s*\)//g;
    $text =~ s/\bpub\b//g;
    $text =~ s/(?:(?:\$crate|\b[A-Za-z_]\w*)\s*::\s*)+//g;
    $text =~ s/,//g;
    return $text =~ /[A-Za-z0-9_]+|\S/g;
  }

  # `| … | { e }` -> `| … | e` when the block holds no `;` of its own (one inside `(…)` or
  # `[…]` is not its own). rustfmt drops the braces of a nested block too
  # (`|x| { { x } }` -> `|x| x`), hence the repeat.
  sub closure_bodies {
    my @t = @_;
    while (1) {
      my (@stack, %drop);
      for my $i (0 .. $#t) {
        if ($t[$i] =~ /^[{(\[]$/) {
          push @stack, [$i, $t[$i] eq "{" && $i > 0 && $t[$i - 1] eq "|", 0];
        }
        elsif ($t[$i] eq ";" && @stack) { $stack[-1][2] = 1 if $t[$stack[-1][0]] eq "{" }
        elsif ($t[$i] =~ /^[})\]]$/ && @stack) {
          my ($open, $closure, $semi) = @{ pop @stack };
          @drop{$open, $i} = () if $closure && !$semi;
        }
      }
      last unless %drop;
      @t = @t[grep { !exists $drop{$_} } 0 .. $#t];
    }
    return @t;
  }

  # Cuts tokens into top-level items: one ends at the `}` that brings the depth back to 0,
  # or at a `;` at depth 0. What is left at the end is an item of its own.
  sub items {
    my @items;
    my @cur;
    my $depth = 0;
    for my $t (@_) {
      push @cur, $t;
      if ($t eq "{") { $depth++ }
      elsif ($t eq "}") { $depth-- if $depth > 0; }
      if (($t eq "}" && $depth == 0) || ($t eq ";" && $depth == 0)) {
        push @items, join " ", @cur;
        @cur = ();
      }
    }
    push @items, join " ", @cur if @cur;
    return @items;
  }

  my %count;    # token -> base minus head
  my %where;    # token -> side -> { file -> n }
  my %icount;   # item -> base minus head
  my %iwhere;   # item -> side -> { file -> n }
  local $/ = "\0";
  while (defined(my $head = <STDIN>)) {
    chomp $head;
    my $src = <STDIN>;
    $src = "" unless defined $src;
    chomp $src;
    my ($side, $file) = split /\t/, $head, 2;
    my @tokens = closure_bodies(normalize($file, $src));
    for my $t (@tokens) {
      $count{$t} += $side eq "base" ? 1 : -1;
      $where{$t}{$side}{$file}++;
    }
    for my $i (items(@tokens)) {
      $icount{$i} += $side eq "base" ? 1 : -1;
      $iwhere{$i}{$side}{$file}++;
    }
  }

  my $bad = 0;
  for my $i (sort keys %icount) {
    my $n = $icount{$i};
    next if $n == 0;
    my $side = $n > 0 ? "base" : "head";
    my $files = join ", ", map { "$_ ($iwhere{$i}{$side}{$_})" } sort keys %{ $iwhere{$i}{$side} };
    my $shown = length $i > 150 ? substr($i, 0, 150) . "..." : $i;
    printf "%s: %d x item in %s: %s\n", $side eq "base" ? "removed" : "added", abs $n, $files, $shown;
    $bad = 1;
  }
  exit 0 unless $bad;
  # The items differ; the token surplus below pinpoints what changed in them.
  for my $t (sort keys %count) {
    my $n = $count{$t};
    next if $n == 0;
    my $side = $n > 0 ? "base" : "head";
    my $files = join ", ", map { "$_ ($where{$t}{$side}{$_})" } sort keys %{ $where{$t}{$side} };
    printf "%s: %d more `%s` than %s, in %s\n",
      $side eq "base" ? "removed" : "added", abs $n, $t,
      $side eq "base" ? "added" : "removed", $files;
  }
  exit 1;
' <"$tmp/records"; then
  echo "not a move: the items above are on one side only" >&2
  exit 1
fi

# --- 5: the test count ---------------------------------------------------------------------
# The base is built in a worktree with a target directory of its own. Sharing HEAD's does not
# work: cargo names the package's artifacts by a hash that leaves out its path, so the two
# checkouts write the same files and the second build, whose sources are older than the
# first one's output, is taken as fresh and skipped. The base's directory sits inside HEAD's
# so its dependencies stay cached between runs.
target=${CARGO_TARGET_DIR:-$PWD/target}
mkdir -p "$target"
target=$(cd "$target" && pwd)
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
echo "move-only ok: same items in $(echo "$files" | wc -l | tr -d ' ') files, $head_tests tests on both sides"
