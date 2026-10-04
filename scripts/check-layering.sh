#!/usr/bin/env bash
# Keep the dependency arrows pointing one way.
#
# The crate is one stack of modules. `RANK` lists the modules of the restructure from the
# bottom up: a module may name only the ones below it. From `task` up it may also name the
# `BOTTOM` modules, which the restructure has not reached yet; `registry` and `mail` may not,
# so neither can reach `session`, and `infra` and `kernel` name nothing outside the RANK
# modules below them (`infra` names nothing outside itself, `kernel` only `infra`). The
# modules the restructure has not reached yet are the `BOTTOM` group, and the commands
# (`TOP`) sit over all of it and may name anything. Nothing below the top names the top.
#
# As long as this passes, splitting into crates later stays a mechanical move: files across,
# a Cargo.toml each, `crate::infra::` -> `adjutant_infra::`.
#
# Runs under the bash 3.2 that macOS ships: no associative arrays, no `grep -P`, no `\s`.
set -uo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.." || exit 1

# The restructure, bottom first. A module here may name only modules earlier in this list.
# One from `task` up may also name the BOTTOM modules; `registry` and `mail` may not, so
# neither can reach `session`, and `infra` and `kernel` may not either. Entries that do not
# exist yet are reservations and are not required to have a source file.
RANK=(infra kernel registry mail task gate jules lifecycle board transport)
# The old bottom: `session` (`Session`, `RepoHub` and the rest), until they move. Answers
# questions using nothing but the standard library, its own input and `infra`.
BOTTOM=(session)
# The top. May name anything; nothing else may name it. `lib` and `main` are the crate roots.
# `cli_args` is a reservation like the RANK ones.
TOP=(cmd mcp cli_args lib main)
# `testing` is `#[cfg(test)]` scaffolding in lib.rs, not a layer: it ships in no binary, so
# naming it says nothing about the direction of the arrows at run time.
ANYWHERE=(testing)
# Names that say nothing about what a file is for.
FORBIDDEN_FILES=(usecase.rs util.rs common.rs)

status=0
fail() {
  echo "$1"
  status=1
}

# in_list NAME LIST... -> 0 when NAME is one of LIST.
in_list() {
  local needle=$1 x
  shift
  for x in "$@"; do
    [ "$x" = "$needle" ] && return 0
  done
  return 1
}

# rank_of NAME -> index in RANK, or nothing.
rank_of() {
  local i=0 x
  for x in "${RANK[@]}"; do
    if [ "$x" = "$1" ]; then
      echo "$i"
      return
    fi
    i=$((i + 1))
  done
}

# The files of module NAME: src/NAME.rs and everything under src/NAME/.
module_files() {
  case "$1" in
    lib | main) [ -f "src/$1.rs" ] && echo "src/$1.rs" ;;
    *)
      [ -f "src/$1.rs" ] && echo "src/$1.rs"
      [ -d "src/$1" ] && find "src/$1" -name '*.rs' -type f | sort
      ;;
  esac
  return 0
}

# Every module the crate has on disk, so one that no list names cannot go unchecked.
modules_on_disk() {
  {
    for f in src/*.rs; do
      [ -f "$f" ] && basename "$f" .rs
    done
    for d in src/*/; do
      d=${d%/}
      d=${d#src/}
      case "$d" in bin) continue ;; esac
      [ -n "$(find "src/$d" -name '*.rs' -type f | head -n 1)" ] && echo "$d"
    done
  } | sort -u
}

# A shim is a file of nothing but `//!` lines, blank lines and `pub use` declarations
# (a declaration may run over several lines). It names the modules it re-exports, and
# that is its whole job.
is_shim() {
  awk '
    { code = $0; sub(/[[:space:]]*\/\/.*$/, "", code) }
    in_use { if (code ~ /;[[:space:]]*$/) in_use = 0; next }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*\/\/!/ { next }
    /^[[:space:]]*pub(\([^)]*\))?[[:space:]]+use[[:space:]]/ {
      if (code !~ /;[[:space:]]*$/) in_use = 1
      next
    }
    { bad = 1; exit }
    END { exit bad }
  ' "$1"
}

# Prints `LINE<TAB>NAME` for every module reference in FILE, one per reference. A whole-line
# comment is skipped before lines are numbered; a trailing `// ...` is kept, because cutting
# at `//` would cut string literals such as URLs and could hide a reference after one.
# `crate::{` and `crate::Upper` (the crate root) print the name `{root}`.
# DEPTH is how many `super::` reach the crate root from the file (1 for src/NAME.rs and
# src/NAME/mod.rs, 2 for src/NAME/foo.rs, ...): that many `super::` count as `crate::`, so
# `super::NAME` names a module of the crate when NAME is one; fewer name items of the file's
# own module and are not references. More than DEPTH can only come from an inline module such
# as `mod tests`, where it reaches the root, so it counts too. Left over: inside an inline
# module exactly DEPTH supers name the file's own module, flagged only if a crate-level
# module has that name.
references() {
  awk -v depth="$2" -v mods=" $3 " '
    /^[[:space:]]*\/\// { next }
    {
      rest = $0
      while (match(rest, /(^|[^A-Za-z0-9_])(crate::|(super::)+)[A-Za-z0-9_{]?[a-z0-9_]*/)) {
        m = substr(rest, RSTART, RLENGTH)
        rest = substr(rest, RSTART + RLENGTH)
        if (m !~ /^(crate|super)/) m = substr(m, 2)
        if (m ~ /^crate::/) {
          name = substr(m, 8)
          if (name == "" || name ~ /^[A-Z{]/) name = "{root}"
          print NR "\t" name
        } else {
          k = 0
          while (substr(m, 1, 7) == "super::") { k++; m = substr(m, 8) }
          if (k >= depth && m != "" && index(mods, " " m " ") > 0) print NR "\t" m
        }
      }
    }
  ' "$1"
}

# --- Forbidden file names ---------------------------------------------------------------
for bad in "${FORBIDDEN_FILES[@]}"; do
  for f in $(find src -name "$bad" -type f | sort); do
    fail "$f: name the file after what it holds, not \`${bad%.rs}\`"
  done
done

# --- Every module is listed, every listed module that must exist does ------------------
on_disk=$(modules_on_disk)
for name in $on_disk; do
  if ! in_list "$name" "${RANK[@]}" "${BOTTOM[@]}" "${TOP[@]}"; then
    fail "src/$name: not in any list in scripts/check-layering.sh; add it to the one it belongs to"
  fi
done
for name in "${BOTTOM[@]}"; do
  if [ -z "$(module_files "$name")" ]; then
    fail "$name is listed in scripts/check-layering.sh but has neither src/$name.rs nor files under src/$name/"
  fi
done

# --- References ---------------------------------------------------------------------------
all_modules="$(echo "$on_disk" | tr '\n' ' ') ${RANK[*]} ${TOP[*]} ${ANYWHERE[*]}"

# RANK modules up to `mail` may not name the BOTTOM modules.
registry_rank=$(rank_of registry)
mail_rank=$(rank_of mail)
for name in $on_disk; do
  # TOP wins over RANK wins over BOTTOM.
  if in_list "$name" "${TOP[@]}"; then
    group=top
  elif [ -n "$(rank_of "$name")" ]; then
    group=rank
  elif in_list "$name" "${BOTTOM[@]}"; then
    group=bottom
  else
    continue # reported above
  fi
  [ "$group" = top ] && continue
  my_rank=$(rank_of "$name")

  for file in $(module_files "$name"); do
    # Below registry a shim is checked too: a `pub use` is still a name.
    if is_shim "$file" && ! { [ -n "$my_rank" ] && [ "$my_rank" -lt "$registry_rank" ]; }; then
      continue
    fi
    # The `super::`s that reach the crate root: one per path component of the file's module.
    rel=${file#src/}
    rel=${rel%.rs}
    rel=${rel%/mod}
    sup=1
    while [ "$rel" != "${rel#*/}" ]; do
      rel=${rel#*/}
      sup=$((sup + 1))
    done
    hits=""
    while IFS="$(printf '\t')" read -r line target; do
      [ -z "$line" ] && continue
      why=""
      if [ "$target" = "$name" ] || in_list "$target" "${ANYWHERE[@]}"; then
        continue
      elif [ "$target" = "{root}" ]; then
        why="names the crate root; write one \`use crate::<module>::...\` per module"
      elif in_list "$target" "${TOP[@]}"; then
        why="names the top (\`$target\`); only cmd, mcp, cli_args, lib.rs and main.rs may"
      else
        t_rank=$(rank_of "$target")
        case "$group" in
          rank)
            if [ -n "$t_rank" ]; then
              [ "$t_rank" -lt "$my_rank" ] || why="names \`$target\`, which is not below \`$name\` in RANK"
            elif [ "$my_rank" -lt "$registry_rank" ]; then
              why="names \`$target\`; below registry, infra names nothing outside itself and kernel names only infra"
            elif [ "$my_rank" -le "$mail_rank" ]; then
              why="names \`$target\`; registry and mail may not name the BOTTOM modules (${BOTTOM[*]})"
            elif ! in_list "$target" "${BOTTOM[@]}"; then
              why="names \`$target\`, which is not in RANK"
            fi
            ;;
          bottom)
            [ "$target" = infra ] ||
              why="names \`$target\`; a bottom module may name only infra"
            ;;
        esac
      fi
      [ -n "$why" ] && hits="$hits$file:$line: $why
"
    done <<EOF
$(references "$file" "$sup" "$all_modules")
EOF
    if [ -n "$hits" ]; then
      printf '%s' "$hits"
      status=1
    fi
  done
done

if [ "$status" -eq 0 ]; then
  echo "layering ok"
fi
exit "$status"
