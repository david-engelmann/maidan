#!/usr/bin/env bash
# Load / soak harness runner (Cluster 198, Arc D).
#
# Drives concurrent REST traffic (post message / read thread / search) at the
# server and prints per-op latency percentiles + throughput — the baseline the
# rest of Arc D's optimizations are measured against. The measurement lives in
# the `#[ignore]`d `load_baseline` test (crates/maidan-server/tests/loadgen.rs);
# this script just sets the env knobs and runs it in release mode.
#
# Usage:
#   # in-process server (SQLite), defaults (8 workers × 50 iterations):
#   scripts/loadgen.sh
#
#   # tune concurrency + switch to a timed soak:
#   MAIDAN_LOADGEN_CONCURRENCY=32 MAIDAN_LOADGEN_DURATION_SECS=60 scripts/loadgen.sh
#
#   # point at a live/scaled deployment (bring your own ids + bearer):
#   MAIDAN_LOADGEN_URL=http://localhost:8080 \
#     MAIDAN_LOADGEN_BEARER=<token> \
#     MAIDAN_LOADGEN_IDS='<workspace>|<channel>|<thread>|<member>' \
#     scripts/loadgen.sh
#
# Env knobs (all optional except IDS when targeting a URL):
#   MAIDAN_LOADGEN_CONCURRENCY   worker tasks (default 8)
#   MAIDAN_LOADGEN_OPS           iterations per worker when not timed (default 50)
#   MAIDAN_LOADGEN_DURATION_SECS timed soak; overrides OPS when > 0 (default 0)
#   MAIDAN_LOADGEN_URL           external base URL (default: in-process server)
#   MAIDAN_LOADGEN_BEARER        bearer token for the external URL
#   MAIDAN_LOADGEN_IDS           workspace|channel|thread|member for the external URL
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== maidan loadgen ==="
echo "concurrency=${MAIDAN_LOADGEN_CONCURRENCY:-8} ops=${MAIDAN_LOADGEN_OPS:-50} duration_secs=${MAIDAN_LOADGEN_DURATION_SECS:-0} url=${MAIDAN_LOADGEN_URL:-<in-process>}"

exec cargo test --release -p maidan-server --test loadgen -- --ignored --nocapture
