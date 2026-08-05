# Open work

Aggregate of deferred items across retros plus standing risks — the
“if I had two hours” backlog. For exhaustive partials and Slack parity,
see [[Remaining Work]].

Updated at each cluster retro. **Baseline:** code on `main` at **`v143.0.0`** (Product Ladder 102+ complete at `v120` / `maidan-scale-1.0`; post-gate hardening 121+). Reconciled against code at v126 (Cluster 127) and again at v143 (Cluster 144).

## Standing risks (still open)

- **Workspace-flat authorization (no channel/thread scope)** — capabilities are workspace-scoped only; any token with `message:post` can read/write *any* channel or thread in its workspace, including private ones (private-channel enforcement exists only in the event-subscription fan-out, `subscribe_grants.rs`, not on REST/MCP read/write). No `channel_members` table. Blocks least-privilege for multi-tenant agent fleets; **flagship of the post-v155 hardening program** (channel/thread RBAC + optional Postgres RLS defense-in-depth).
- **At-most-once event bus (default path)** — transactional outbox (**10**), quarantine (**12**), HTTP outbox replay (**56**); NOTIFY duplicates/gaps possible on the optimistic path. **Mitigated:** opt-in `at_least_once` reconcile delivery (WebSocket **125**, MCP SSE **126**) is gap-free + at-least-once per `consumer_id`.
- **Bootstrap / `AUTH_DISABLED`** — high-impact misconfiguration; compile-time strip (**91**) removes the path in hardened (`--no-default-features`) builds.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener** — best-effort recovery; `/health/ready` reflects errors.
- **SQLite semantic search** — brute-force cosine fallback; optional `sqlite-vec` feature for an index; HNSW is Postgres-only (by design, not a gap).
- **`hash-v1` default** — `openai-compatible` provider (v117) gives real semantics; `hash-v1` is the offline/dev default, not semantically meaningful.
- **`rsa` advisory `RUSTSEC-2023-0071`** — ignored (RS256 id_token verify via openidconnect v4; no fixed `rsa`); clears on openidconnect v5 (unreleased). See [Dependencies.md](Dependencies.md).
- **No `v93`–`v100` tags** — clusters 93–101 shipped as one batch (PR #264), released as `v101.0.0`; not a backlog. All four gate tags (incl. `maidan-operator-1.0`) are cut.

## Shipped (reference)

| Ladder / tag | Highlights |
|--------------|------------|
| **17–27** | MCP fan-out, SQLite semantic, Helm server, purge, streamable subset |
| **35–58** | `maidan-2.0` product gate — DMs, webhooks, slash, FSM, erase, quotas, completion e2e |
| **59–67** | [[Agent Integration]], streamable TTL, A2A card, outbox ops, app OAuth, context |
| **68–76** | Automation DLQ, capability map, vault truth, A2A subscribe, MCP context, agent gate — [[Retros/Cluster 76.0]] |

**Release signing:** cosign keyless `sign-blob --bundle` over tarballs + SBOM in `release.yml` (automated; was previously manual).

## Still deferred (no separate owner)

| What | Notes |
|------|-------|
| Multi-region active-active | Out of scope |

_Closed (verified v126/v131/v132/v144/v148): OpenAPI↔capability map (**121**), OTLP export + dashboards + e2e (**89/90/123**), `sqlite-vec` + per-model embedding tables (**85/86**); webhook+automation delivery unification — substantially addressed (shared signing/backoff + unified operator API; storage intentionally separate, **131**); global cross-workspace admin audit query API (`GET /operator/audit`, gated by `audit:read-global`, **132**); docs link-checker in CI (`mdbook-linkcheck` gate, **144**); full MCP streamable transport spec-completeness (version negotiation + header + batching + notifications + GET SSE + `Accept` + resumability + server→client requests, arc **145–148**)._

## Known state

- **Latest tag:** **`v156.0.0`** (post-gate hardening, Phase XXIV). All four gate tags cut (`maidan-2.0` v58, `maidan-agent-1.0` v76, `maidan-operator-1.0` v101, `maidan-scale-1.0` v120).
- **Active work:** post-gate hardening clusters (121+); no further ladder gate defined. See [[Roadmap]] + [[Remaining Work]].
- **Integrators:** start at [[Agent Integration]] and `contracts/`.

## How to read this file

- **[[Remaining Work]]** — partial implementations + Slack matrix.
- **[[Roadmap]]** — cluster pointer and historical closes.
- Retro PRs are the right time to add or remove deferrals.
