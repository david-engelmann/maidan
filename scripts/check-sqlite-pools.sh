#!/usr/bin/env bash
# Every SQLite pool outside maidan-search/src opens through
# `maidan_search::sqlite_pool_options` (or `_with`), so each connection loads
# sqlite-vec and the per-connection PRAGMAs (#1614). Tests too (#1725): a bare
# `SqlitePoolOptions::new()` in crates/*/src or crates/*/tests fails this
# check, unless the line above it says `// sqlite-pools: bare on purpose`
# (a test of what a bare pool does).
set -euo pipefail
cd "$(dirname "$0")/.."
hits=$(rg -n -B1 'SqlitePoolOptions::new\(' crates/*/src crates/*/tests \
  --glob '!crates/maidan-search/src/**' || true)
bad=$(printf '%s\n' "$hits" | awk '
  /^--$/ { prev = ""; next }
  /SqlitePoolOptions::new\(/ && prev !~ /sqlite-pools: bare on purpose/ { print }
  { prev = $0 }
')
if [ -n "$bad" ]; then
  echo "bare SqlitePoolOptions::new(); use maidan_search::sqlite_pool_options():" >&2
  echo "$bad" >&2
  exit 1
fi
echo "sqlite pools: ok"
