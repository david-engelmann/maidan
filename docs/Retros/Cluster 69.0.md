# Cluster 69.0 retro — Capabilities matrix complete

> Tag **`v69.0.0`**.

## What shipped

- `contracts/mcp-capability-map.json` aligned with `maidan_mcp::tools::required_capability`.
- `contracts/http-capability-routes.json` for table-driven HTTP denial samples.
- `mcp_capability_matrix_e2e`: deny + allow gate for every MCP tool in the catalog.
- Extended `capability_matrix_e2e` with HTTP contract denials; CI via `check-agent-contract.sh`.
- [[Capability Map]] documents full MCP tool table and contract file pointers.

## What was deferred

- Exhaustive OpenAPI path ↔ capability map (sample HTTP contract only).
- Positive HTTP allow matrix (MCP allow gate covers tools; HTTP uses deny-only contract).
- Generated capability map from source (hand-maintained JSON for now).

## Forward look

Cluster **70**: Vault truth pass ([[Architecture]], [[Remaining Work]], stale rows).
