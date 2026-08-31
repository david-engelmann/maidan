# Cluster 332.0 retro — MCP artifact tenant isolation (audit P0.1)

> Tag **`v332.0.0`**. Phase XXIV (post-gate hardening). **Cluster 1 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The one **P0** the 2026-08-30 full-repo audit surfaced: the MCP artifact tools bypassed the
Cluster-204 per-workspace artifact isolation that the REST path enforces. Fixed.

- **`get_artifact_metadata`** (MCP tool) now takes `auth` and gates on
  `artifact_ref_exists(auth.workspace_id, sha)` before `get_artifact_by_sha`, returning
  `McpError::NotFound` when the caller's workspace has no access ref — indistinguishable from a
  genuinely-absent artifact (no cross-tenant existence oracle), exactly as REST `get_artifact` does.
- **`resources/read maidan://artifacts/{sha}`** (server `resources_read`) gains the same ref gate
  (the `artifacts` arm was the one branch of that match with no access check — threads/channels were
  already gated by Cluster 161). `resources::read` now reports `byte_length` from `meta.size_bytes`
  instead of loading the whole blob just to measure it.
- **MCP uploads** (`upload_artifact` + `complete_artifact_multipart`) now record the per-workspace
  access ref via `record_artifact_ref` (a shared `record_ref` helper), so an MCP-uploaded blob is
  fetchable by its own workspace (previously it recorded no ref → REST fetch 404'd) and is isolated
  from others.

## Surprises / decisions

- **The read leak was the real bug; the write-404 was the documented half.** The Cluster-330 retro
  noted only the write-side missing ref; the audit found the *read* side (`get_artifact_metadata` +
  `resources/read`) did no ref check at all — a `workspace:read` bearer could read any tenant's
  artifact metadata + byte-length by SHA. Correcting the record: `resources/read` returns metadata +
  `byte_length`, **not** the raw bytes, so this was a metadata + existence + dedup-oracle leak, not
  full blob disclosure — still P0 on the primary agent transport.
- **Used `record_artifact_ref` (standalone), not `upsert_artifact_with_event`.** The security fix is
  the ref; switching MCP uploads to the event path is the broader P1.1 write-path-parity work. Keeping
  332 to `upsert_artifact` + `record_ref` scopes it tightly to the isolation hole; eventing/bus-notify
  parity for all MCP write tools is the next cluster.
- **`threads`/`channels` resource reads were already safe** — `resources_read` runs
  `ensure_thread_access`/`ensure_channel_access` (Cluster 161); only the `artifacts` arm was open. The
  audit's focus on artifacts was correct.

## Test evidence

`mcp_artifact_tools_enforce_tenant_isolation` (two workspaces: A uploads; B is denied on both
`get_artifact_metadata` and `resources/read maidan://artifacts/{sha}` with `NotFound`; A is allowed on
both). Full `maidan-mcp` lib suite (57) + MCP contract-sync + `mcp_capability_matrix_e2e` +
`artifact_isolation_e2e` (REST) green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Next in the audit program: **P1.1 MCP write-path parity** (migrate the 8 event-less MCP write tools to
`*_with_event` + publish; `edit_message` first — it silently breaks the flagship as-of replay + embedding
reindex) → P1.2 unify the context assembler → P1.3 `whoami` + `initialize` instructions → P1.4 post-path
round-trips → P1.5 egress wire tests + LSN replica CI → P2 docs/polish.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the post-flagship audit program
([[Open Work]]).
