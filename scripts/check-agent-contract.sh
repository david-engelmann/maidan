#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/check-build-bin-dir.sh
cargo test -p maidan-types --test event_kinds_contract
cargo test -p maidan-types --test event_surface_disposition_contract
cargo test -p maidan-types --test ws_subscribe_filter_schema
cargo test -p maidan-server --test it portable_frame_goldens::
./scripts/check-mcp-resource-notifications.sh
cargo test -p maidan-mcp --test it tools_catalog_contract::
cargo test -p maidan-mcp --test it mcp_capability_map_contract::
cargo test -p maidan-server --test it http_capability_map_contract::
cargo test -p maidan-server --test it http_operation_kinds_contract::
cargo test -p maidan-server --test it http_openapi_capability_map_contract::
cargo test -p maidan-server --test it http_capability_matrix_e2e::
./scripts/check-release-records.sh
