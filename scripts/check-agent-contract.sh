#!/usr/bin/env bash
# The agent contract: the golden files under contracts/ and the tests that pin
# them, plus the release records.
#
#   scripts/check-agent-contract.sh                  everything (local use)
#   scripts/check-agent-contract.sh --no-cargo       only the checks that are not tests
#   scripts/check-agent-contract.sh --nextest-filter print the tests as a nextest filterset
#
# CI runs the tests once, from the shared nextest archive (the integration
# shards run them in full; a feature or docs-only PR runs this filter), so the
# lint job takes --no-cargo instead of compiling three test binaries again.
# CONTRACT_TESTS is the one list both paths read.
set -euo pipefail
cd "$(dirname "$0")/.."

# <package> <module>: every test under <module>:: in the package's `it` binary.
CONTRACT_TESTS=(
  "maidan-types event_kinds_contract"
  "maidan-types event_surface_disposition_contract"
  "maidan-types ws_subscribe_filter_schema"
  "maidan-server portable_frame_goldens"
  "maidan-mcp tools_catalog_contract"
  "maidan-mcp mcp_capability_map_contract"
  "maidan-server http_capability_map_contract"
  "maidan-server http_operation_kinds_contract"
  "maidan-server http_openapi_capability_map_contract"
  "maidan-server http_capability_matrix_e2e"
)

mode="${1:-all}"
case "$mode" in
  --nextest-filter)
    filter=""
    for entry in "${CONTRACT_TESTS[@]}"; do
      read -r pkg module <<<"$entry"
      filter+="${filter:+ | }(binary_id(${pkg}::it) & test(/^${module}::/))"
    done
    printf '%s\n' "$filter"
    exit 0
    ;;
  --no-cargo | all) ;;
  *)
    echo "usage: $0 [--no-cargo | --nextest-filter]" >&2
    exit 2
    ;;
esac

./scripts/check-build-bin-dir.sh
if [ "$mode" = all ]; then
  for entry in "${CONTRACT_TESTS[@]}"; do
    read -r pkg module <<<"$entry"
    cargo test -p "$pkg" --test it "${module}::"
  done
fi
./scripts/check-mcp-resource-notifications.sh
./scripts/check-release-records.sh
