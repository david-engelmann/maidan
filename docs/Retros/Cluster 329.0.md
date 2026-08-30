# Cluster 329.0 retro — immutable context snapshot artifact

> Tag **`v329.0.0`**. Phase XXIV (post-gate hardening). **Cluster 11 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

Freeze an assembled context pack into the **existing** content-addressed artifact store —
a tamper-evident record of exactly what the agent was handed, deduped by sha256, ref-guarded
per Cluster 204.

- **`POST /threads/:id/context/snapshot`** (`tools/thread.rs`) — builds the thread context
  pack (live or `as_of`, via the same `ThreadContextQuery` params), serializes it to JSON,
  `artifacts.put`s the bytes, and `upsert_artifact_with_event`s the row + the per-workspace
  ref + `ArtifactUpserted` event (the same atomic path as a normal upload). Returns the
  `Artifact` (`sha256`, `size_bytes`, `kind=context_snapshot`, `mime=application/json`).
  Gated `artifact:upload` + `ensure_thread_access`.
- **`ArtifactKind::ContextSnapshot`** — new kind (`as_str`/`parse`/`default_mime` +
  serde `rename_all`); `default_mime` = `application/json`.
- The snapshot is fetchable at `GET /artifacts/:sha` and **deduped** — an identical pack
  yields the same sha (one blob). Re-ask can later attach it by sha (the seed `pack`
  inclusion, deferred).

## Surprises / decisions

- **The kind `CHECK` constraint bit.** `maidan_artifacts.kind` has a `CHECK (kind IN (…))`
  (migration 0007), so the first insert of `context_snapshot` failed with a bare "database
  error" (500). Fix: migration **pg `0055` / sqlite `0054`** widens the allowlist — Postgres
  `DROP`/`ADD CONSTRAINT`, SQLite a table rebuild (same columns as 0007; `maidan_artifact_refs`
  has no FK to the table, so the rebuild is safe). **Lesson: a new enum variant that lands in
  a `CHECK`-constrained column needs a migration, not just the Rust arm.**
- **Reused the artifact store wholesale** — no new blob path, no new store method; the
  snapshot is "just an artifact" with a dedicated kind. Content-addressing gives dedup +
  tamper-evidence for free ("prefix paid once, N angles").
- **No `RefSide::Artifact` lineage edge.** `RefSide` is Thread/Message; a thread→artifact
  edge would be a wider change. The snapshot's provenance is its `uploaded_by`/`created_at` +
  the sha; the re-ask link is the future seed `pack` inclusion. Kept out of scope.
- **`artifact:upload` cap + thread access** — the freeze creates an artifact (so
  `artifact:upload`) and reads thread content (so `ensure_thread_access`); the two gates
  compose, mirroring `upload_artifact` + the content check.

## Test evidence

`context_snapshot_e2e` (auth-enabled: freeze → `201` + `context_snapshot` artifact; fetch the
blob → it IS the pack; identical snapshot dedups to the same sha; read-only token → `403`);
both-backend store roundtrips (the 0054/0055 migrations apply + artifact roundtrip);
`openapi_e2e` bijection + `http_capability_matrix_e2e` (new route denied without
`artifact:upload`) + types green. fmt + strict clippy + `--all-targets` + bootstrap-strip
clean; mdbook linkcheck green.

## Forward look

The immutable snapshot is done (REST). The arc's remaining tail is all optional: an MCP
snapshot tool, the seed `pack` inclusion (attach a snapshot sha), a `WorkSeeded` signal, and
**item 7 flow/setup template** (likely declined as already covered by export/import 187 +
269–270). After that the flagship arc is complete — a good point to open a research round.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
