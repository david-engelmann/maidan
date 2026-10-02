#!/usr/bin/env bash
# Scripts that build a binary and then run it must follow CARGO_TARGET_DIR.
# cargo honors that variable; a hardcoded ./target/debug does not, so the
# script looks beside the repo for a binary that was written somewhere else.
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0
for script in scripts/sdk-test.sh scripts/lease-demo.sh; do
  if grep -n '\./target/debug' "$script"; then
    echo "$script still runs ./target/debug" >&2
    fail=1
  fi
  if ! grep -q 'bin_dir="${CARGO_TARGET_DIR:-target}/debug"' "$script"; then
    echo "$script does not take its binaries from CARGO_TARGET_DIR" >&2
    fail=1
  fi
done
[[ $fail -eq 0 ]] && echo "build bin dir: ok"
exit $fail
