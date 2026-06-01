#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p maidan-types --test event_kinds_contract
cargo test -p maidan-mcp --test tools_catalog_contract
