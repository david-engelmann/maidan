# Cluster 216.0 — RLS spike (decision ADR); Program A complete

**Theme:** Program A (security & correctness round 2), part 15 (final) — the
Row-Level Security spike, delivered as a decision.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v216.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| RLS assessment ADR (deferred; app-layer RBAC authoritative) | `docs/Decisions.md` (new `## Security` section) |
| Program A marked complete | `docs/Open Work.md`, `docs/Roadmap.md` |

## Why

Program A's last item was a **spike**: evaluate Postgres Row-Level Security (RLS)
as database-enforced tenant isolation beneath the app-layer RBAC (the Cluster
160–165 arc). A spike's deliverable is a decision + rationale, not necessarily
code — and here the assessment lands on **defer**, so the cluster is the ADR that
records it.

## The finding

RLS keys row visibility on a per-connection session GUC (`SET LOCAL
app.current_workspace`), but Maidan's architecture doesn't support that cheaply:

- The `PgPool` is a shared pool with no per-request tenant binding (its only
  `after_connect` step is `statement_timeout`).
- `Store` methods are workspace-agnostic (they take entity ids); RLS needs the
  workspace context at query time, which means threading a request context through
  every method × both backends and wrapping every read in a GUC-setting transaction.
- SQLite has no RLS, so it would be Postgres-only — an asymmetry against the
  enforced dual-backend parity.
- The bearer/orchestrator model is cross-workspace by design; a single
  `current_workspace` GUC fights it.
- It duplicates an already-comprehensive, tested control (RBAC on every read/write/
  event/management/reference/artifact/search/federation surface).

**Decision: defer.** App-layer RBAC stays authoritative. The ADR records the design,
the blockers, and concrete trigger conditions for revisiting (a compliance mandate;
a `Store` per-request context arriving for another reason; or Postgres becoming the
sole backend).

## Exit criteria

- The RLS spike is resolved with a recorded decision — **met**.
- **Program A (security & correctness round 2, Clusters 202–216) is complete.**
- `v216.0.0` tagged.

## Verification & limits

- Docs-only cluster; the `docs` + `mdbook-linkcheck` CI jobs validate the new ADR
  section renders and links cleanly. No code change, so behaviour is unchanged.
- **Limit:** this is a *decision*, not an implementation — if a trigger condition in
  the ADR later holds, RLS becomes a real cluster starting from `maidan_messages` +
  `maidan_channels`.

## References

- [[Retros/Cluster 216.0]]; [[Decisions]] (`## Security`). Program: [[Roadmap]] +
  memory `maidan-next-arc-program`. Concludes Program A; Programs B/C/D follow.
