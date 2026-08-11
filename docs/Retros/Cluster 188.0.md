# Cluster 188.0 retro — a metering number that won't melt Prometheus

> Tag **`v188.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc B (multi-tenant SaaS operability), part 4.

## What shipped

- `WorkspaceUsage` (members/channels/threads/messages) + `Store::workspace_usage`
  (one scalar-subquery aggregate, both backends), exposed at
  `GET /workspaces/:id/usage` (`workspace:read`).

## Surprises / decisions

- **The "obvious" per-tenant metric is the wrong one.** Labeling hot-path
  counters with `workspace_id` gives per-tenant series but unbounded cardinality
  — it degrades the whole Prometheus as tenants grow. A DB-computed on-demand
  snapshot is low-cardinality and a *better* billing basis (exact counts, not
  sampled rates). This was the real design call, and it's the opposite of what
  the "add per-tenant metrics" framing suggests.
- **Content-addressed storage has no clean per-tenant number.** Artifacts are
  deduped across workspaces (no `workspace_id`), so "storage bytes for tenant X"
  is genuinely ambiguous — attributing by uploader double-counts a shared blob.
  Omitted on purpose (tracked) rather than shipping a misleading number.
- **`?1` / `$1` reuse keeps it one bind.** SQLite named-positional `?1` and
  Postgres `$1` both let the four subqueries share a single workspace-id bind —
  one round trip, no four-way rebind.

## Capability table extension

| Change | Where |
|--------|-------|
| `GET /workspaces/:id/usage` (workspace:read) + `Store::workspace_usage` | `maidan-types` + `maidan-store` + `routes/workspace.rs` |

## Risks identified + still open

- **Net additive, read-only.** Open (Open Work): no storage bytes (content-address
  dedup makes it ill-defined); point-in-time snapshot only (no historical
  time-series — an operator samples on their cadence); the message count JOINs
  three tables (fine on demand, not for a hot path).

## Forward look

Arc B closes with a secret-rotation keyring (189), then Arc C (agentic
task-queue depth) and Arc D (performance & scale).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
