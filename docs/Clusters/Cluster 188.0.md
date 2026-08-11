# Cluster 188.0 — SaaS ops: per-workspace usage / metering

**Theme:** Arc B (multi-tenant SaaS operability), part 4 — a per-tenant usage
signal for metering / quota visibility, without a Prometheus cardinality blow-up.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v188.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `WorkspaceUsage` type + `Store::workspace_usage` (one aggregate query, both backends) | `maidan-types/src/usage.rs`, `maidan-store/src/{sqlite,postgres}/workspaces.rs` |
| `GET /workspaces/:id/usage` (workspace:read) | `routes/workspace.rs`, `app.rs`, OpenAPI + `contracts/http-capability-map.json` |

## Why

Metrics were fixed-cardinality — no per-tenant signal an operator could meter or
bill on. The obvious fix (a `workspace_id` label on hot-path counters like
`http.server.request_total`) is a **cardinality time-bomb**: one series per
workspace per metric, unbounded as tenants grow, which degrades the whole
Prometheus.

## The fix

A per-workspace **usage snapshot computed on demand** from the DB — members,
channels, threads, messages (excluding tombstoned rows) — exposed at
`GET /workspaces/:id/usage`. It stays low-cardinality (a per-request aggregate,
not a scraped per-tenant series) and is the natural metering/billing basis. One
query per call with scalar subqueries (bind the workspace id once).

- **Auth `workspace:read`** — aggregate counts are low-sensitivity metadata, not
  content, and a member seeing their own workspace's totals is reasonable.
- **Artifact storage omitted (deliberate).** Blobs are content-addressed and
  deduped **across** workspaces (no `workspace_id` on `maidan_artifacts`), so
  per-tenant bytes is ill-defined; attributing by uploader would double-count
  shared blobs. Tracked in Open Work.

## Exit criteria

- A caller can read live member/channel/thread/message counts for a workspace,
  scoped and tombstone-excluding — **met**.
- `v188.0.0` tagged.

## Verification & limits

- `maidan-store` `workspace_usage` test (SQLite + Postgres-testcontainers): counts
  are scoped to the workspace (a second workspace's content doesn't leak) and
  exclude a tombstoned message. `openapi_e2e` bijection green (route + body schema
  + capability map).
- Limits: no storage bytes (see above); the message count JOINs three tables —
  fine for an on-demand admin/metering call, not a hot path; no historical
  time-series (this is a point-in-time snapshot — an operator samples it on their
  own cadence).

## References

- [[Retros/Cluster 188.0]]; `maidan-types/src/usage.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc B).
