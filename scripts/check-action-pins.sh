#!/usr/bin/env bash
# Every `uses:` in a workflow or composite action names a full commit SHA and
# carries its version as a trailing comment. A tag or a branch is a pointer
# its owner can move: release.yml holds packages:write and the cosign
# id-token, so one moved tag in a third-party action would sign and ship an
# image. The comment is what Dependabot reads and rewrites when it moves a pin
# (.github/dependabot.yml), and what a reviewer reads to know the version.
#
# Allowed: `owner/repo[/path]@<40 hex> # <version>`, a local action
# (`./path`), and `docker://image@sha256:<64 hex>`.
set -euo pipefail
cd "$(dirname "$0")/.."

files=()
while IFS= read -r f; do files+=("$f"); done < <(
  git ls-files -- '.github/workflows/*.yml' '.github/workflows/*.yaml' \
    '.github/actions/**/action.yml' '.github/actions/**/action.yaml'
)
if [ "${#files[@]}" -eq 0 ]; then
  echo "action pins: no workflow files found; refusing to pass" >&2
  exit 1
fi

pinned='^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*("|'"'"')?([^@[:space:]"'"'"']+@[0-9a-f]{40}("|'"'"')?[[:space:]]+#[[:space:]]*[^[:space:]]+.*|\./[^[:space:]]*|docker://[^[:space:]]+@sha256:[0-9a-f]{64}("|'"'"')?([[:space:]]+#.*)?)[[:space:]]*$'

bad=0
total=0
while IFS= read -r hit; do
  total=$((total + 1))
  line=${hit#*:*:}
  if ! printf '%s\n' "$line" | grep -qE "$pinned"; then
    echo "$hit" >&2
    bad=$((bad + 1))
  fi
done < <(grep -nE '^[[:space:]]*(-[[:space:]]+)?uses:' "${files[@]}")

if [ "$total" -eq 0 ]; then
  echo "action pins: found no uses: lines; refusing to pass" >&2
  exit 1
fi
if [ "$bad" -ne 0 ]; then
  echo "action pins: $bad of $total uses: lines above are not pinned to a full commit SHA with a '# vX.Y.Z' comment" >&2
  echo "resolve the SHA with: gh api repos/<owner>/<repo>/commits/<tag> --jq .sha" >&2
  exit 1
fi
echo "action pins: ok ($total uses: lines pinned)"
