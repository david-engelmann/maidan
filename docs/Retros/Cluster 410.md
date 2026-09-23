# Cluster 410 retro — accountable usage and bounded authorization evidence

> Post-gate hardening · `v410.0.0` · umbrella #984 · PRs #988/#989/#991 + close record

## Outcome

Wave 4 row #41 is closed. Usage reporting is now an economic write rather than
an unaccounted counter bump: the active lease holder reports an immutable price
snapshot, Maidan derives the workspace payer, and one transaction binds the
ledger row, accumulated budget, durable event, and any over-budget claim
failure. REST and MCP share those semantics. Authorization decisions now have
a separate content-free observability lane whose metric dimensions are bounded
and whose detailed denial warnings are sampled.

| Slice | Evidence | Result |
|-------|----------|--------|
| 410.1–410.2 | #988; dual-backend store suites | `PayerStamp`, `UsageReported`, idempotent ledger writes, stale-lease fencing, and budget enforcement land atomically. |
| 410.3 | #989; REST/MCP/OpenAPI contract tests | Both public surfaces derive reporter and payer identity, return the original outcome on exact retry, and reject forged identity or conflicting reports. |
| 410.4 | #991; authorization observability e2e + promtool fixture | REST and MCP use one content-free decision record; aggregate labels are fixed-cardinality, detail is sampled 1-in-64, and sustained denials alert. |

## Decisions

- Money is stored as integer micro-USD, matching the existing budget unit. The
  roadmap's `usd_minor` shorthand did not justify introducing lossy cents.
- The reporter supplies the immutable price snapshot and Maidan verifies its
  arithmetic. This is accountable evidence, not a vendor rate card or invoice.
- Reporter and payer are derived authority: the active claim holder and the
  thread's workspace. Neither is accepted from the request body.
- An exact `report_id` retry reads its original outcome and emits no second
  event. Different content under the same id and a stale lease fail before any
  total changes.
- Authorization denials remain aggregate observability, not durable
  `maidan_audit` rows. This preserves Cluster 182's attacker-controlled
  write-amplification decision.
- Principal and resource IDs may appear in content-free structured traces, but
  never in Prometheus labels. Metrics carry only surface, known capability,
  outcome, and resource kind.

## What surprised us

- The released quickstart image predates newly added binary flags, so a source
  command can be impossible against the very artifact it names. Validation has
  to execute the published artifact, not infer behavior from current source.
- `UsageReported` made an exhaustive federation remap fail to compile. The
  right boundary is explicit rejection: economic events are local accounting
  evidence and must not be imported as peer-authored spend.
- The repository's docs and release checks use `rg`, but fresh GitHub runners
  did not provide it. The dependency had to be installed explicitly in every
  workflow that invokes those scripts.
- Prometheus exposition preserves metric-label insertion order. The parity e2e
  therefore locks the actual bounded shape rather than assuming alphabetical
  rendering.

## Residual risk and follow-up

- A reporter-provided price snapshot can be internally consistent while still
  differing from a provider's eventual invoice. Rate-card administration,
  entitlements, and billing reconciliation remain out of scope.
- This cluster records authorization identity and outcome in an operational
  lane; it does not turn denials into durable database rows. Cluster 411 must
  extend the same record with delegated subject and grant identity while
  preserving that bound.
- `member:impersonate` remains until Cluster 411 replaces caller-chosen acting
  identity with explicit, short-lived delegation grants and then removes the
  capability.

## Release ledger

| Item | Value |
|------|-------|
| Tag | `v410.0.0` |
| Roadmap | Wave 4 row #41 closed |
| Database compatibility | Additive Postgres 0102 / SQLite 0101 usage-ledger migration |
| API compatibility | Additive REST `POST /threads/:id/usage`, MCP `report_usage`, and `UsageReported` event |
| Runtime behavior | Accepted usage atomically charges the budget; exact retries are reads; denials emit bounded aggregate observability |
| New release gates | Dual-backend idempotency/fencing, REST/MCP parity, fixed-label metric e2e, and denial-rate promtool fixture |
