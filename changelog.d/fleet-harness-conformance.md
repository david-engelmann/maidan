### Fleet harness profiles get a conformance check

- **Added:** `GET /workspaces/{id}/usage-conformance` reads a workspace's newest completed usage reports against a published harness profile (`harness`, `harness_version`, `provider`, `model`, `boot_pack_sha256`, each optional) and reports per member how many conform and which values diverge (C9). `workspace:read`, the caller's workspace only. The store gains `list_workspace_usage_ledger` on both backends. No migration.
