#!/usr/bin/env bash
# Model-check the TLA+ specs in specs/tla with TLC.
#
# Each spec runs with its config and must pass. Each spec also has a config
# that turns off one mechanism (the old claim deadline handling, the peer's
# chain check); TLC must find the named invariant violated there, which shows
# the invariant catches the bug it is meant to catch.
#
# Usage:  scripts/tla.sh
# Env:    TLA2TOOLS_JAR (reuse a tla2tools.jar; it must match the pinned hash)
#         JAVA (default: java)
set -euo pipefail

cd "$(dirname "$0")/../specs/tla"

version="v1.7.4"
sha256="936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
java="${JAVA:-java}"
jar="${TLA2TOOLS_JAR:-${TMPDIR:-/tmp}/tla2tools-${version}.jar}"

if [[ ! -f "$jar" ]]; then
  curl -fsSL -o "$jar.part" \
    "https://github.com/tlaplus/tlaplus/releases/download/${version}/tla2tools.jar"
  mv "$jar.part" "$jar"
fi
echo "${sha256}  ${jar}" | sha256sum -c --quiet

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

tlc() {
  "$java" -XX:+UseParallelGC -cp "$jar" tlc2.TLC -workers auto -deadlock \
    -metadir "$work/states" -config "$2" "$1"
}

pass() {
  echo "== $1 ($2): must pass"
  tlc "$1" "$2" | tee "$work/out"
  grep -q "No error has been found" "$work/out"
}

violates() {
  echo "== $1 ($2): must violate $3"
  if tlc "$1" "$2" >"$work/out"; then
    cat "$work/out"
    echo "expected TLC to find $3 violated" >&2
    exit 1
  fi
  if ! grep -q "Invariant $3 is violated" "$work/out"; then
    cat "$work/out"
    echo "expected TLC to find $3 violated" >&2
    exit 1
  fi
  echo "found, as expected"
}

pass Claim.tla Claim.cfg
pass EventLog.tla EventLog.cfg
violates Claim.tla ClaimNoReset.cfg NoLeaseNoDeadline
violates EventLog.tla EventLogUnordered.cfg ShreddedIsUnreadable
