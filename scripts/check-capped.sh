#!/usr/bin/env bash
# scripts/capped must stop a command at its time cap (124) and its memory cap
# (137), and otherwise pass the command's own status through.
set -euo pipefail
cd "$(dirname "$0")/.."
status() { set +e; "$@" >/dev/null 2>&1; echo $?; set -e; }
fail() { echo "check-capped: $*" >&2; exit 1; }
[ "$(status scripts/capped --time 2 -- sleep 30)" = 124 ] || fail "time cap did not stop a sleeper with 124"
[ "$(status scripts/capped --mem-gb 0.05 -- python3 -c 'b = bytearray(300 * 1024 * 1024); import time; time.sleep(30)')" = 137 ] \
  || fail "memory cap did not stop a 300 MB allocator with 137"
[ "$(status scripts/capped -- sh -c 'exit 3')" = 3 ] || fail "the command's own status was not passed through"


# Bad flags must fail closed before a runaway starts.
[ "$(status scripts/capped --time 0 -- true)" != 0 ] || fail "time 0 was accepted"
[ "$(status scripts/capped --time -1 -- true)" != 0 ] || fail "negative time was accepted"
[ "$(status scripts/capped --mem-gb 0 -- true)" != 0 ] || fail "mem-gb 0 was accepted"
[ "$(status scripts/capped --mem-gb -1 -- true)" != 0 ] || fail "negative mem-gb was accepted"
echo "check-capped: ok"
