#!/usr/bin/env bash
# Every SQLite pool outside maidan-search opens through
# `maidan_search::sqlite_pool_options` (or `_with`), so each connection loads
# sqlite-vec and the per-connection PRAGMAs (#1614). A bare
# `SqlitePoolOptions::new()` in crates/*/src fails this check.
set -euo pipefail
cd "$(dirname "$0")/.."
if hits=$(rg -n 'SqlitePoolOptions::new\(' crates/*/src --glob '!crates/maidan-search/**'); then
  echo "bare SqlitePoolOptions::new() outside maidan-search; use maidan_search::sqlite_pool_options():" >&2
  echo "$hits" >&2
  exit 1
fi
echo "sqlite pools: ok"
