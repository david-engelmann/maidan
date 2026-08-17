# Cluster 224.0 retro — the queue gets a gauge

> Tag **`v224.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 8.

## What shipped

- `GET /channels/:cid/queue-depth` → `{ open, ready, assigned, blocked }`, a
  point-in-time partition of a channel's open task threads, backed by a single
  aggregate query (`Store::channel_queue_depth`) on both backends.

## Surprises / decisions

- **The predicate was already written — twice.** `ready` had to mean *exactly* what
  `claim_next` would take, or the number lies. Rather than re-derive it, I copied
  `claim_next`'s claimability clause (unassigned-or-lease-expired + deps-terminal)
  into the aggregate. The one wrinkle: `claim_next` picks a single row (`LIMIT 1`
  with an `ORDER BY`), while queue-depth counts *all* matching rows — same `WHERE`
  predicate, different projection. Keeping them textually identical is the
  correctness contract; a future change to claimability must touch both.
- **`COALESCE(SUM(...), 0)`, not bare `SUM`.** `SUM` over zero rows is `NULL`, so an
  empty channel would deserialize a null into an `i64` and panic. Each partition
  count is wrapped; `COUNT(*)` is already 0-safe. The empty-channel case is the
  first assertion in the store test precisely because it's the easy one to get
  wrong.
- **Aggregate, not a metric.** Cluster 188 already reasoned this out for
  `workspace_usage`: a per-channel Prometheus label is a cardinality bomb, and an
  orchestrator wants exact counts at decision time, not a scraped rate. Queue-depth
  is the same shape, so it's the same answer — an on-demand DB aggregate.
- **The new-route checklist held.** OpenAPI path stub + `paths(...)` + a
  `components(schemas(QueueDepth))` reg + the `http-capability-map` GET entry, and
  `openapi_e2e` (bijection) + `http_capability_matrix_e2e` both stayed green. A GET
  needs no matrix body clause, so this was the light version of the preflight.

## Capability table extension

| Change | Where |
|--------|-------|
| `GET /channels/:cid/queue-depth` + `Store::channel_queue_depth` | `routes/channel.rs`, `store/*/threads.rs` |

## Risks identified + still open

- **Point-in-time only** — lease expiry advances with wall-clock, so successive
  calls can differ; inherent to the question, documented on the `QueueDepth` type.

## Forward look

The task-queue subsystem now has its observability read. Next: the MCP
`get_queue_depth` tool (225, the REST/MCP split), then the larger Program B lanes —
scheduled/recurring tasks, a capability registry + skill routing, and coordination
waits + structured results. Then Programs C (notifications & reach) and D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 223.0]].
