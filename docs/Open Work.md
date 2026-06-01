# Open work

Aggregate of everything deferred across retros plus standing risks.
The "if I had two hours, what could I work on" backlog.

**Post–Product Ladder 17–27:** see [[Remaining Work]] for the exhaustive
remaining-issues, Slack-parity, and partial-implementation register.

Updated at the close of each cluster or optional minor retro. Items move
from "open" to "shipped" when the owning release merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. **`v10.0.0`** added transactional outbox
  so commit and enqueue happen together; a relay publishes after commit (relay
  retries may duplicate NOTIFY). **Cluster 12.0** adds max-attempts quarantine for poison rows. `maidan_events` + replay HTTP API shipped in
  Cluster D; WS/MCP auto-replay on lag shipped in **`v3.0.0`** when
  `filter.workspace_id` is set; reconnect uses signed `resume_token`
  (**`v4.0.0`**); `replay_truncated` signals when one replay window is insufficient.
- **Bootstrap flags are high-impact.** Bootstrap routes are now gated by
  `MAIDAN_BOOTSTRAP=1` when auth is enabled (`v1.4.1`), but leaving
  `AUTH_DISABLED` or bootstrap flags on outside controlled seed windows
  still creates avoidable exposure.
- **Indexer staleness is opt-in.** Set `INDEXER_STALE_SECS` to mark
  `/health/ready` degraded when the indexer has not observed an event
  recently. Default `0` disables the check.
- **PostgresBus listener recovery is best-effort.** `/health/ready` reports
  `bus: error` while the background task is in a retry loop (`v1.1.0`); it
  clears after the next successful `recv`.
- **Coverage depth is still modest.** CI enforces an **11.0%** line floor
  (**`v11.0.0`**); optional Codecov upload when `CODECOV_TOKEN` is set.
  Further incremental uplift is opportunistic.
- **SQLite semantic search** ships at `v18.0.0` with brute-force cosine over stored
  embeddings (no HNSW); `sqlite-vec` SQL functions deferred (sqlx linkage).
- **`hash-v1` is not semantic.** Real provider support shipped in `v1.3.0`,
  but default deployments may still run deterministic `hash-v1` if not configured.

## Shipped post-1.0

| Release / area | Highlights |
|----------------|------------|
| **Product ladder 17–27** | `v17.0.0`–`v27.0.0` — MCP fan-out, SQLite semantic, multipart S3, router, A2A RPC, capabilities matrix, UI, Helm, workspace purge, completion gate, MCP streamable HTTP subset — see [[CHANGELOG]] and [[Remaining Work]] §1 |
| Tracks T–X | See [[Post-1.0]] (closure #121) |
| **`v12.0.0`** | Outbox quarantine + max attempts — [[Retros/Cluster 12.0]] |
| **`v11.0.0`** | Coverage floor 11%, outbox/relay tests — [[Retros/Cluster 11.0]] |

**Still manual:** Sigstore/cosign of release artifacts (V.3 — [[Operations]]).

## Recently closed: Product Ladder 17–27

Clusters **17–27** merged in PR #198 (code on `main`; tags `v23.0.0`–`v27.0.0`
await retro + tag cut). Highlights:

- **23** — `/ui` product tabs (events, search, thread FSM, tokens)
- **24** — `helm/maidan` + template smoke in CI
- **25** — `POST /workspaces/:id/purge` + audit
- **26** — [[Product Completion Checklist]] + gate e2e
- **27** — `POST /mcp/streamable` (response + notifications on one SSE body)

Before that: **`v22.0.0`** capabilities — [[Retros/Cluster 22.0]].

## Agent substrate (owned ladder)

Post–**`v67.0.0`** deferrals from [[Clusters/Product Ladder 59+]] are scheduled in
[[Clusters/Product Ladder 68+]] (Clusters **68–76**, product gate **`maidan-agent-1.0`**).

**Recently closed:** **`v68.0.0`** — slash/FSM HTTP delivery ledger, worker, operator replay ([[Retros/Cluster 68.0]]).
**Recently closed:** **`v69.0.0`** — MCP capability map + table-driven matrix e2e ([[Retros/Cluster 69.0]]).

## Still deferred (no owner yet)

| What | Notes |
|------|-------|
| Per-model embedding tables / mixed dimensions | 5.0 filters by model at query time; table split deferred |
| `sqlite-vec` / HNSW on SQLite | 18.0 brute-force path; extension linkage deferred |
| Schema parity property test (`information_schema`) | Cluster A retro |
| Score normalization across Postgres vs SQLite ranks | Documented in 5.0; unification deferred |
| SQLite file-backed durability tests | Cluster V retro |
| Full MCP streamable HTTP (bidirectional session) | 27.0 shipped subset; spec session mux still open — [[Remaining Work]] |
| Full workspace GDPR erasure | 25.0 message purge only — [[Remaining Work]] |
| Helm umbrella (Postgres + MinIO + server) | 24.0 server chart only |
| `SendStreamingMessage` (A2A) | Cluster 21.0 retro |
| Postgres `mcp-stdio` | Cluster H retro |
| Slack parity gaps (DMs, edit, pins, presence, …) | [[Remaining Work]] §4 |
| Outbox quarantine replay API | Cluster 12.0 retro |
| OTLP dashboards / SLOs | Track T |
| Multi-region active-active | Ladder 17–27 out of scope |

**Shipped (removed from deferrals):** S3 multipart (`v19.0.0`), MCP HTTP notifications (`v16.0.0`), MCP resource fan-out (`v17.0.0`), Helm server chart (`v24.0.0`), workspace message purge (`v25.0.0`), message router (`v20.0.0`), A2A `SendMessage`/`GetTask` (`v21.0.0`).

## Known state at this handoff

- **Latest tag:** **`v28.0.0`** (privacy complete).
- **Active cluster:** **none** — pick work from [[Remaining Work]].
- **Docs site:** mdBook on `main`; enable GitHub Pages in repo settings if not live.

## How to read this file

- **[[Remaining Work]]** — exhaustive post-ladder backlog (partials, Slack matrix, suggestions).
- The "Standing risks" list at the top is the always-on register.
- [[Post-1.0]] is the live minor-release ladder; this file is the backlog.
- A retro PR is the legitimate moment to add deferred items.
