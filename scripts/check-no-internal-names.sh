#!/usr/bin/env bash
# No file in the public tree names an internal project, team or repository.
# The names live in scripts/internal-names.txt (one per line, matched
# case-insensitively); this script fails when one comes back anywhere else.
# The list itself is the one file allowed to contain them.
set -euo pipefail
cd "$(dirname "$0")/.."

list="scripts/internal-names.txt"
[[ -f "$list" ]] || { echo "internal names: $list is missing" >&2; exit 1; }

names=()
while IFS= read -r line; do
  line="${line%%#*}"
  line="$(xargs <<<"$line" || true)"
  [[ -n "$line" ]] && names+=("$line")
done < "$list"
[[ "${#names[@]}" -gt 0 ]] || { echo "internal names: $list names nothing" >&2; exit 1; }

# One -e per name, matched literally (-F): a name is a name, not a pattern,
# so a dot or a bracket in one cannot widen or break the match.
args=()
for name in "${names[@]}"; do args+=(-e "$name"); done
# Only exit 1 means "no matches". Anything higher is the scan itself
# failing (a bad pathspec, no work tree), and a check that fails open is
# worse than no check, so refuse to pass.
status=0
hits="$(git grep -inIF "${args[@]}" -- . ":(exclude)${list}")" || status=$?
if [[ $status -gt 1 ]]; then
  echo "internal names: the scan failed (git grep exited $status); refusing to pass" >&2
  exit 1
fi
if [[ -n "$hits" ]]; then
  echo "internal names: the public tree names an internal project again:" >&2
  echo "$hits" >&2
  echo "internal names: use a neutral name, or cite the contract by its title" >&2
  exit 1
fi
echo "internal names: none of the ${#names[@]} listed names appear in the tree"
