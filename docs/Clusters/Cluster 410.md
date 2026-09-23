# Cluster 410 — Wave 4 #41: PayerStamp ledger and authorization audit lane

> Post-gate hardening · target tag `v410.0.0` · umbrella issue #984

## Contract

- Turn `report_usage` into an accountable ledger write. A report names a unique
  id, active claim lease, model, input/output/cache token tiers, immutable price
  snapshot, and integer micro-USD charge. The authenticated active holder is
  the reporter and the thread's workspace is the payer; callers cannot select
  either identity.
- Make economic retries idempotent. Repeating the same report is a read of its
  original outcome; reusing its id with different content or from a stale lease
  fails before totals change.
- Atomically bind the accepted ledger row, accumulated budget usage,
  non-federatable `UsageReported` event, and—when a cap binds—the existing
  claim release, `ClaimFailed`, and DLQ entry.
- Emit content-free REST and MCP authorization records with principal, action,
  outcome, and resource. Denials stay in a bounded observability lane, not the
  durable `maidan_audit` table, preserving Cluster 182's write-amplification
  decision. Metric labels remain fixed-cardinality.

## Vocabulary decisions

- The roadmap's `usd_minor` shorthand resolves to the existing API/storage unit
  `usd_micros`. Introducing integer cents beside `max_usd_micros` would make
  budget arithmetic lossy and ambiguous.
- A price snapshot is reporter-supplied evidence whose arithmetic Maidan
  validates and preserves. It is not a Maidan rate card or proof that a vendor
  will invoice the same amount.
- The authorization lane never captures request/response bodies, prompts,
  completions, tool arguments, messages, secrets, or provider payloads.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 410.1 | #988 | Types, dual-backend ledger/idempotency foundation, `UsageReported`, and atomic budget enforcement with stale-lease fencing |
| 410.2 | folded into 410.1 | The economic write and its enforcement invariant shipped atomically rather than exposing an unfenced intermediate store API |
| 410.3 | #989 | REST/MCP parity, authenticated reporter and workspace payer derivation, OpenAPI/MCP contracts, public docs |
| 410.4 | #991 | Shared content-free REST/MCP decision records, fixed-cardinality aggregate metric, sampled denial detail, and sustained-rate alert |
| 410.close | #992 | Ledgers, retrospective, and `v410.0.0` tag |

## Exit criteria

- Exact retries do not increment totals or append a second event; conflicting
  retries and stale leases fail closed.
- Every accepted increment has one queryable ledger row and one durable
  `UsageReported` event carrying the same PayerStamp.
- Budget exhaustion remains a claim failure, never a successful thread close.
- REST and MCP parity, both stores, event contracts, content-exclusion rules,
  fixed-cardinality metrics, and denial-rate bounds have executable evidence.

## Non-goals

- Invoicing, entitlements, payment processing, rate-card administration, or
  live provider-price discovery.
- Per-workspace Prometheus labels or retroactive pricing of old events.
- Durable per-denial rows in `maidan_audit`.
- Any capture of model or workplace content.
