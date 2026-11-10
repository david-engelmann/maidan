# WIP: `kind` filter for claim-next and workspace queue depth

Drafts only; nothing here is compiled. Delete this directory when the work lands.

- `kind_store_tests.rs`: append to `crates/maidan-store/tests/review_family.rs` once #2186 merges (reuses its `Room`, `Pool`, `change_into_review`, `exclusive`, `REPO`; add `QueueDepth` to its imports).
- `kind_e2e_tests.rs`: append to `crates/maidan-server/tests/workspace_queue_depth_e2e.rs` (reuses `filtered_depth`, `call_tool`, `tool_json`, `get`).
- `kind_goldens.py mcp`: adds `kind` to the catalog, `tools_list_*` and `profile-worker` goldens (round-trip checked).
- `kind_goldens.py openapi docs.json`: adds `ClaimNextThread.kind` and the `kind` query param; `docs.json` holds the doc-comment strings joined with `\n`.
- `kind_reference.py`: rewrites the three tools' sections of `book/src/mcp-reference.md` from the catalog (a no-op on today's catalog).
