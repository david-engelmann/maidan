### A coordinator MCP profile

- **Added:** `POST /mcp/coordinator` serves a fixed, sorted `tools/list` of 36 tools for filing, finding, tracking, steering and cancelling tasks, reading results and deliveries, catching up and replying, reviews, and gate decisions, the same bytes for every caller like `/mcp/worker` and `/mcp/reviewer`. A client that coordinates no longer has to load the full catalog. `listings/hol-guard/mcp.maidan-coordinator.json` is generated for it.
