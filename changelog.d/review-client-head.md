### A verdict can name the head it reviewed

- **Fixed:** `submit_review` (REST and MCP) takes an optional `head_sha`, the full commit SHA the reviewer read. A head the change has since moved past is refused with 409 instead of stamping the verdict with a head the reviewer never saw (#1574).
