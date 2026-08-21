#!/usr/bin/env bash
# Chaos / fault-injection harness runner (Cluster 259, Program D).
#
# Drives a stream of publishes at a PostgresBus while periodically killing the
# LISTEN backend connection, then asserts every published event still reached the
# local broadcast — proving the Cluster-258 self-healing NOTIFY floor back-fills
# what the dropped notifications would have delivered. Needs Docker (the test
# spins up a Postgres testcontainer) and is timing-sensitive, so the scenario is
# `#[ignore]`d and this script runs it explicitly.
#
# Usage:
#   # defaults (50 ops, kill the listener every 10 ops, 50ms between ops):
#   scripts/chaos.sh
#
#   # a heavier soak with more frequent faults:
#   MAIDAN_CHAOS_OPS=200 MAIDAN_CHAOS_KILL_EVERY=25 MAIDAN_CHAOS_DELAY_MS=25 \
#     scripts/chaos.sh
#
# Env knobs (all optional):
#   MAIDAN_CHAOS_OPS         events to publish (default 50)
#   MAIDAN_CHAOS_KILL_EVERY  kill the LISTEN backend every N ops; 0 disables (default 10)
#   MAIDAN_CHAOS_DELAY_MS    delay between ops in ms (default 50)
set -euo pipefail

cd "$(dirname "$0")/.."

echo "chaos: ops=${MAIDAN_CHAOS_OPS:-50} kill_every=${MAIDAN_CHAOS_KILL_EVERY:-10} delay_ms=${MAIDAN_CHAOS_DELAY_MS:-50}"
cargo test -p maidan-bus --test chaos --release -- --ignored --nocapture
