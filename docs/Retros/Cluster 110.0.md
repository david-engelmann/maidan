# Cluster 110.0 retro — Per-workspace fairness

> Tag **`v110.0.0`**. Fifth and final cluster of Phase XX (hot-path hardening); **closes the phase**.

## What shipped

- **Per-workspace rate limit** — `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` / `_WINDOW_SECS` cap a workspace's *total* request rate across all its tokens, on `/workspaces/{wid}/…` routes (incl. search), keyed `ws:{wid}` through the existing fixed-window limiter + Redis-optional backend. Independent of the per-client global limit; both default off. `workspace_id_from_path` extracts the wid with a cheap path split (no auth/routing dependency, stays pre-auth on the hot path). (110.0.1, #306)
- **Noisy-neighbor test** — `tenant_fairness_e2e`: workspace A capped at its limit (429) while workspace B keeps returning 200 — the exit criterion. In its own test binary so the env doesn't bleed into the other rate-limit tests. (110.0.3, #306)
- **Docs** — `docs/Production.md` "Tenant fairness" section + the two env rows; `docs/Threat-Model.md` T8 (resource-exhaustion / DoS-by-tenant). (110.0.4, this PR)

## What was deferred / not covered

- **Per-workspace indexer throughput budget (110.0.2) → Cluster 116.** The plan frames the indexer budget as the *policy* half pairing with Cluster 116's batch-embedding-pipeline *mechanism* half; building the throttle before the batch pipeline would mean reworking it. The cluster's exit criterion — a workspace at its limit can't degrade another's **search/write latency** — is met and tested by the request-path limiter. The indexer-fairness slice is recorded against 116 in [[Open Work]].
- **Hard CPU/IO isolation** between tenants (separate instances / Postgres resource groups) — infra-level, explicit non-goal.
- **Billing / usage accounting** — non-goal.

## Surprises

- **The workspace dimension has to be cheap and pre-auth.** The limiter middleware runs before auth/route extraction, so it can't read the workspace from `AuthContext`. Pulling the wid straight from the path prefix (`/workspaces/{wid}/…`) keeps the check a string split with no DB/auth dependency — and naturally covers the search path, the headline concern. Routes without a wid in the path (`/channels/...`, `/threads/...`) fall back to the per-client limit; that's an acceptable seam for v1 fairness.

## Decisions

- **Two independent limiter dimensions, both opt-in, both default off.** The global per-client limit and the per-workspace fairness limit compose (a request must pass both when both are enabled), reusing one fixed-window primitive and the same Redis-optional backend — no new infrastructure. See `docs/Production.md` (Tenant fairness) and `docs/Threat-Model.md` T8.

## Capability table extension

| Capability | Where |
|------------|-------|
| Per-workspace request-rate fairness | `rate_limit::middleware`, `MAIDAN_WORKSPACE_RATE_LIMIT_MAX`, key `ws:{wid}` |
| Noisy-neighbor regression guard | `tenant_fairness_e2e` |

## Risks

- **Routes without `wid` in the path** aren't workspace-limited (only per-client). Workspace-scoped writes under `/channels`/`/threads` rely on the global limit; tightening that seam (resolve workspace post-auth) is future work if abuse is seen there.
- A too-tight per-workspace budget throttles a legitimate large tenant — defaults are off and the docs steer toward generous values.

## Phase XX — closed

Phase **XX (hot-path hardening)** is complete at **`v110.0.0`**: bulk context reads (106), configurable DB pool & timeouts (107), adaptive outbox relay (108), ANN tuning + search bench (109), and per-workspace fairness (110). Next: Phase **XXI — correctness & coverage** (Clusters 111–115), opening with the `maidan-auth` test suite (111) and FSM property tests (112).
