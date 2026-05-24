# Open work

Aggregate of everything deferred across the five completed retros
plus standing risks. The "if I had two hours, what could I work on"
backlog.

Updated at the close of each cluster. Items move from "open" to
"shipped" when the cluster that owns them merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. `maidan_events` + replay HTTP
  API shipped in Cluster D, but subscribers must poll replay on gap —
  no automatic WS backfill on lag beyond `replay_hint` (v1.1.2); subscribe
  `after_id` replays from `maidan_events` on connect (v1.1.3).
- **Bootstrap routes are unauthenticated.** `POST /workspaces` and
  `POST …/members` have no Bearer gate; production must seed with
  `AUTH_DISABLED` then mint tokens before enabling auth.
- **Indexer staleness is opt-in.** Set `INDEXER_STALE_SECS` to mark
  `/health/ready` degraded when the indexer has not observed an event
  recently. Default `0` disables the check.
- **`v0.1.0` GitHub Release didn't auto-create.** Cleanup PR landed
  (#36 → `macos-13` for x86_64 darwin). Verify `v0.3.0` tag triggers
  a successful release before considering this resolved.
- **PostgresBus listener recovery is best-effort.** `/health` reports
  `bus: error` while the background task is in a retry loop; it clears
  after the next successful `recv`.
- **No coverage gate in CI.** Local + integration tests run, but no
  `≥ N%` threshold is enforced. → Cluster T (when
  `cargo-llvm-cov` lands as a CI job).
- **SQLite has no semantic search.** `Search::semantic_search`
  returns `Unsupported`. → Cluster F+ candidate via `sqlite-vec` if
  the extension's sqlx integration matures.

## Specific items deferred to a later cluster

### To Cluster T (telemetry + perf)

| What                                                              | Source                |
|-------------------------------------------------------------------|-----------------------|
| Coverage upload (`cargo-llvm-cov` + codecov)                      | Cluster A retro       |
| OTLP exporter, structured JSON logs (shipped T.1)                 | Track T               |
| Request-id middleware + HTTP spans (shipped H + T.1)                | —                     |
| Indexer heartbeat on `/health/ready` (shipped T.2)                  | Track T               |
| SQLite `journal_mode = WAL` + `busy_timeout` PRAGMA tuning        | Cluster A retro       |
| Schema parity property test diffing `information_schema` rows     | Cluster A retro       |
| Persistent event log (id-pointer + table fetch, beyond 8KB)       | Cluster B retro       |
| `websearch_to_tsquery` Google-style operators in `q`              | Cluster C retro       |
| Score normalization across dialects (Postgres vs SQLite ranks)    | Cluster C retro       |
| `cargo-cyclonedx` SBOM generation                                 | Cluster B retro       |
| 1000-event WS soak / slow-subscriber stress                       | Cluster B retro       |
| Mutation tests against bus + routes                               | Cluster B retro       |

### Shipped in Cluster D (v0.3.0)

| What                                                  | PRs    |
|-------------------------------------------------------|--------|
| Thread FSM + transition log                           | #48–50 |
| Nested threads + HSM                                  | #51    |
| `hash-v1` embedding indexer (Postgres)                | #52    |
| Persistent event log + replay API                     | #53    |
| MCP `prompts/list` + `prompts/get`                      | #54    |

### Still deferred from Cluster D scope

| What                                                  | Target           |
|-------------------------------------------------------|------------------|
| Real ML embedding model (replace `hash-v1`)           | Post-1.0         |
| Resumable WS subscriptions / reconnection tokens      | Cluster T / F    |
| Per-model embedding tables / dimension variations     | Post-1.0         |
| Faceted search (author / channel / kind filters)      | Cluster T        |
| Automatic subscriber replay on NOTIFY miss            | Cluster T        |

### Shipped in Cluster E (v0.4.0)

| What                                                   | PRs    |
|--------------------------------------------------------|--------|
| `ArtifactKind` + migration 0007                        | #65    |
| S3Store + HTTP + MCP + streaming helpers               | #66    |

### Still deferred from Cluster E scope

| What                                                   | Target           |
|--------------------------------------------------------|------------------|
| S3 multipart for multi-GB blobs                          | Cluster T        |
| Upgrade aws-sdk off `rustls-webpki` 0.101              | When upstream    |

## Cluster 1.0 — complete

See [`docs/Retros/Cluster 1.0.md`](Retros/Cluster%201.0.md). Tag `v1.0.0`.

### To Cluster H (web UI / production polish)

| What                                                          | Source           |
|---------------------------------------------------------------|------------------|
| Web UI (`maidan-web` crate)                                   | Roadmap          |
| MCP stdio transport for desktop clients                       | Cluster B retro  |
| SSE for MCP `resources/subscribe`                             | Cluster B retro  |
| Graceful shutdown                                             | Cluster B retro  |
| Helm chart as an alternative to Kustomize                     | Cluster A plan   |
| Docs site (mdBook / Docusaurus / VitePress) consuming `docs/` | Decisions        |
| Auto-creating a v0.1.0 release retroactively                  | PR #36 body      |

### To Cluster U (performance)

| What                                                 | Source           |
|------------------------------------------------------|------------------|
| `cargo-mutants` mutation suite in nightly CI         | Cluster A retro  |
| `criterion` bench suite                              | Cluster A retro  |
| `bencher.dev` vs `cargo-criterion` decision          | Cluster A plan   |
| HorizontalPodAutoscaler manifest                     | Cluster A retro  |
| Postgres + SQLite EXPLAIN-driven query tuning        | (implicit)       |

### To Cluster V (security + privacy)

| What                                                        | Source           |
|-------------------------------------------------------------|------------------|
| GDPR right-of-erasure flow (hard delete past tombstones)    | Cluster A retro  |
| NetworkPolicy manifests for k8s                             | Cluster A retro  |
| Sigstore signing of release artifacts                       | Cluster A retro  |
| SQLite file-backed durability tests                         | Cluster A retro  |

### To Cluster W (docs)

| What                                                  | Source           |
|-------------------------------------------------------|------------------|
| Coverage upload + report site                         | Cluster A retro  |
| OpenAPI spec generation (utoipa or aide)              | Cluster B retro  |
| Per-route reference docs auto-generated               | (implicit)       |

### To Cluster X (release engineering)

| What                                          | Source        |
|-----------------------------------------------|---------------|
| Auto-create v0.1.0 release retroactively      | PR #36 body   |
| Pin docker image digests in k8s prod overlay  | k8s/README    |

## Known unfinished tasks at this handoff

- **Cluster ladder A–H + 1.0 complete** — latest tag `v1.0.0`. See
  [[Post-1.0]] for track order and optional `v1.1.0` minor.
- **Track T in progress** — T.3 (`cargo-llvm-cov`) optional; federation
  compose smoke shipped (#105).
- **Federation pull-path compose smoke** — deferred; needs
  `remote_workspace_id` or peer model clarification (see [[Tracks/Track T]]).

## How to read this file

- The "Standing risks" list at the top is the always-on register.
  Items leave the list when the underlying issue is fixed.
- The per-cluster sections enumerate items the original PR scoped
  out. Items move from "deferred to" tables into their respective
  cluster's plan when work starts.
- A retro PR is the only legitimate moment to add items here. If
  you spot a deferred item that isn't listed, the previous retro
  missed it — open a follow-up PR that updates this file.
