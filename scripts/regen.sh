#!/usr/bin/env bash
# Regenerate every golden and every file derived from the MCP catalog
# (crates/maidan-mcp/src/tools/catalog.json, the one source). After a merge
# conflict in any of them, take either side and run this.
set -euo pipefail
cd "$(dirname "$0")/.."
UPDATE_PROFILE_GOLDEN=1 cargo test -q -p maidan-mcp --lib profiles
MAIDAN_WRITE_SNAPSHOTS=1 cargo test -q -p maidan-mcp --test wire_snapshots
MAIDAN_WRITE_SNAPSHOTS=1 cargo test -q -p maidan-server --test wire_snapshots
MAIDAN_WRITE_SNAPSHOTS=1 cargo test -q -p maidan-server --test ui_modulepreload_contract
cargo run -q -p maidan-mcp --bin gen-mcp-reference
python3 scripts/regen_derived.py
git status --short
