# Cluster 204.0 — security: cross-tenant artifact isolation

**Theme:** Program A (security & correctness round 2), part 3 — stop a caller in
one workspace from fetching another tenant's artifact blob just by knowing its
SHA-256.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v204.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `maidan_artifact_refs (workspace_id, sha256)` link table + backfill | `migrations/{postgres/0036,sqlite/0035}_artifact_workspace_refs.sql` |
| `record_artifact_ref` / `artifact_ref_exists` (both backends) | `store/{postgres,sqlite}/artifacts.rs`, `store.rs` |
| Ref written on upload; enforced on `get_artifact*` (404 when absent) | `routes/artifact.rs` |
| Migration registration in the runner | `maidan-store/src/migrate.rs` |
| Cross-tenant e2e | `tests/artifact_isolation_e2e.rs` |

## Why

Artifacts are content-addressed and **deduped across workspaces** — there is no
`workspace_id` on `maidan_artifacts` (a deliberate storage-efficiency choice,
noted as far back as Cluster 188). But `GET /artifacts/:sha` and
`/artifacts/:sha/meta` gated only on `workspace:read`, so any authenticated caller
who knew or guessed a SHA-256 could download **another tenant's blob**. Dedup also
made it a known-plaintext oracle: upload a file, observe the dedup, and you've
confirmed it exists in some other workspace. Threat-Model T5 flagged this residual.

## The fix

A `maidan_artifact_refs (workspace_id, sha256)` table records which workspaces may
access each SHA:

- **Write** a ref on every upload (single-shot + multipart complete) for the
  uploader's `auth.workspace_id`.
- **Enforce** on `get_artifact` + `get_artifact_metadata`: require
  `artifact_ref_exists(auth.workspace_id, sha)`. Absent → **404** (not 403), so a
  cross-tenant SHA can't even be confirmed to exist.
- **Dedup preserved**: two workspaces that upload the same bytes each get their
  own ref and both keep access — the blob is still stored once.
- **Backfill** in the migration: link each existing artifact to its uploader's
  workspace so existing workspaces keep access to what they uploaded.
- `bypass` (auth disabled / tests) is unrestricted.

## Exit criteria

- A caller in workspace B cannot fetch (blob or metadata) an artifact workspace A
  uploaded, by SHA — **met**.
- `v204.0.0` tagged.

## Verification & limits

- `artifact_isolation_e2e`: A uploads → A reads blob + meta (200); B (same SHA,
  no ref) gets 404 on both; B re-uploads the same bytes → gets its own ref → 200.
- `artifact_e2e` (bypass) unchanged; `http_capability_matrix`,
  `workspace_purge_artifact_blobs`, `thread_context`, `mcp_e2e`, `ui_collab`,
  `capability_matrix`, store `backend_parity`/`dialect_parity` all green.
- **Gotcha:** migrations are a hardcoded `include_str!` list in `migrate.rs`, not
  auto-discovered — a new `.sql` needs a `const` + an `apply_*` call or it never
  runs (memory `maidan-migration-register`).
- Limits (Open Work): **ref-counted blob GC** is deferred — purging a workspace
  drops its refs (via the FK `ON DELETE CASCADE`) but does not delete the shared
  blob when the last ref goes (a pre-existing dedup+purge concern, not worsened
  here). Backfill covers uploader-linked artifacts; an artifact referenced by a
  workspace that didn't upload it (rare) would need re-upload or a follow-up
  message-metadata backfill.

## References

- [[Retros/Cluster 204.0]]; `routes/artifact.rs`, `migrate.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Closes
  Threat-Model T5.
