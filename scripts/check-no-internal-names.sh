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

pattern="$(IFS='|'; echo "${names[*]}")"
hits="$(git grep -inIE "$pattern" -- . ":(exclude)${list}" || true)"
if [[ -n "$hits" ]]; then
  echo "internal names: the public tree names an internal project again:" >&2
  echo "$hits" >&2
  echo "internal names: use a neutral name, or cite the contract by its title" >&2
  exit 1
fi
echo "internal names: none of the ${#names[@]} listed names appear in the tree"
