#!/usr/bin/env bash
# Self-check for check-build-bin-dir.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/check-build-bin-dir.sh >/dev/null
# A comment-only mention is not a runtime use.
code='# default is under target debug'
code="${code%%#*}"
if [[ "$code" == *'./target/debug'* ]]; then
  echo "comment strip failed" >&2
  exit 1
fi
# A runtime line is detected.
line='./target/'"debug"'/maidan-server'
code="${line%%#*}"
if [[ "$code" != *'./target/debug'* ]]; then
  echo "runtime detect failed" >&2
  exit 1
fi
echo "check-build-bin-dir self-check: ok"
