#!/usr/bin/env bash
# Scripts that build a binary and then run it must follow CARGO_TARGET_DIR.
# cargo honors that variable; a hardcoded ./target/debug does not, so the
# script looks beside the repo for a binary that was written somewhere else.
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0

# Runtime uses only: a line that still runs ./target/debug after stripping a
# trailing comment. Pure comment lines, this checker, and scripts/test_* helpers
# (which mention the needle while asserting) are ignored.
while IFS= read -r hit; do
  file="${hit%%:*}"
  rest="${hit#*:}"
  line="${rest#*:}"
  base="${file##*/}"
  case "$base" in
    check-build-bin-dir.sh|test_*) continue ;;
  esac
  code="${line%%#*}"
  if [[ "$code" == *'./target/debug'* ]]; then
    echo "$hit" >&2
    echo "$file still runs ./target/debug" >&2
    fail=1
  fi
done < <(rg -n '\./target/debug' scripts --glob '*.sh' || true)

# Known runners must also take binaries from CARGO_TARGET_DIR when they build.
for script in scripts/sdk-test.sh scripts/lease-demo.sh; do
  if [[ -f "$script" ]] && ! grep -q 'bin_dir="${CARGO_TARGET_DIR:-target}/debug"' "$script"; then
    echo "$script does not take its binaries from CARGO_TARGET_DIR" >&2
    fail=1
  fi
done
[[ $fail -eq 0 ]] && echo "build bin dir: ok"
exit $fail
