# Cluster 216.0 retro — a spike that ships a decision, and Program A closes

> Tag **`v216.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 15 — the final one.

## What shipped

- The RLS spike, resolved: a Decisions ADR that assesses Postgres Row-Level
  Security and **defers** it, keeping app-layer RBAC authoritative. With it,
  **Program A (202–216) is complete.**

## Surprises / decisions

- **A spike's honest output is sometimes a paragraph, not a patch.** The
  temptation on a security item is to ship *some* code so it "counts." But a
  half-RLS (policies on one table, no GUC plumbing) would either be a no-op
  (permissive policy) or break every query (RLS defaults to deny with no
  `current_workspace` set). The only real options were the full pool + `Store`-
  context refactor or a documented defer — and the refactor's cost/benefit
  (Postgres-only defense-in-depth over an already-comprehensive control) made
  *defer* the correct engineering answer. Writing that down clearly is the
  deliverable.
- **The architecture makes the decision for you.** Three independent facts each
  push toward defer: the shared pool has no per-request tenant binding, the `Store`
  trait is workspace-agnostic, and SQLite has no RLS at all (parity break). Any one
  is a serious blocker; together they're decisive. The ADR names all three so a
  future reader doesn't re-litigate.
- **Trigger conditions matter more than the "no."** The valuable part of a defer
  ADR is *when to revisit*: a compliance mandate, a `Store` per-request context
  arriving for another reason (read-replica routing would bring one), or Postgres
  becoming the sole backend. Those turn "no" into "not yet, and here's the signal."

## Capability table extension

| Change | Where |
|--------|-------|
| RLS assessment ADR (deferred; app-layer RBAC authoritative) | `docs/Decisions.md` |

## Program A retrospective (202–216)

Fifteen clusters. Three real residual vulns closed (session-identity spoofing 202,
DM subscribe/metadata 203, cross-tenant artifacts 204); the full transactional-
outbox refactor (205–214, every domain mutation event-atomic on both backends); a
federation ingest trust policy (215); and this RLS decision (216). The security-led
program the 2026-08-12 sweep set is done.

## Forward look

**Program B — agentic orchestration** is next (task DAG, scheduled/recurring tasks,
capability registry + skill routing, queue-depth, coordination waits + structured
results), then C (notifications & reach) and D (scale & durability). Program B is a
larger feature program than A's hardening work — a good point to scope its first
cluster deliberately.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes Program A, which
began at [[Retros/Cluster 202.0]].
