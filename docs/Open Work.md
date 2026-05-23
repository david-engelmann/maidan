# Open work

Aggregate of everything deferred across the three completed retros
plus standing risks. The "if I had two hours, what could I work on"
backlog.

Updated at the close of each cluster. Items move from "open" to
"shipped" when the cluster that owns them merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. A subscriber that misses a
  notification has no recovery. → Cluster D persistent event log.
- **WS + MCP are anonymous.** Anyone with network access can
  subscribe / call tools. → Cluster F auth.
- **No indexer lag metric on `/health`.** A stuck indexer is
  invisible to operators. → Cluster T.
- **`v0.1.0` GitHub Release didn't auto-create.** Cleanup PR landed
  (#36 → `macos-13` for x86_64 darwin). Verify `v0.2.0` tag triggers
  a successful release before considering this resolved.
- **PostgresBus background listener task lacks supervision.** If the
  listener errors permanently, it sleeps 1s and retries forever —
  but never surfaces the failure to `/health`. → Cluster T.
- **No coverage gate in CI.** Local + integration tests run, but no
  `≥ N%` threshold is enforced. → Cluster T (when
  `cargo-llvm-cov` lands as a CI job).
- **SQLite has no semantic search.** `Search::semantic_search`
  returns `Unsupported`. → Cluster F+ candidate via `sqlite-vec` if
  the extension's sqlx integration matures.

## Cluster D backlog (FSM-driven thread lifecycle)

Cluster D's plan doc has not been written yet. Likely PRs:

- **D.1** Schema 0004: thread state transitions table (`thread_id`,
  `from_state`, `to_state`, `actor_id`, `occurred_at`).
- **D.2** `maidan-fsm` crate: typed state machine for threads
  (`Open` → `InReview` → `Closed` → `Archived`), with the transition
  table from above as the persistent log.
- **D.3** Hierarchical state machine for nested sub-threads (per
  the original scope doc).
- **D.4** Real embedding generation in the indexer (load model at
  boot, generate vectors for `MessagePosted` events).
- **D.5** Persistent event log + replay (resolves the at-most-once
  risk).
- **D.6** MCP `prompts/list` + `prompts/get` (per-thread prompts).
- **D.retro** + tag `v0.3.0`.

## Specific items deferred to a later cluster

### To Cluster T (telemetry + perf)

| What                                                              | Source                |
|-------------------------------------------------------------------|-----------------------|
| Coverage upload (`cargo-llvm-cov` + codecov)                      | Cluster A retro       |
| OTLP exporter, request-id middleware, structured JSON logs        | Cluster A retro       |
| SQLite `journal_mode = WAL` + `busy_timeout` PRAGMA tuning        | Cluster A retro       |
| Schema parity property test diffing `information_schema` rows     | Cluster A retro       |
| Persistent event log (id-pointer + table fetch, beyond 8KB)       | Cluster B retro       |
| Indexer lag metric on `/health`                                   | Cluster C retro       |
| `websearch_to_tsquery` Google-style operators in `q`              | Cluster C retro       |
| Score normalization across dialects (Postgres vs SQLite ranks)    | Cluster C retro       |
| `cargo-cyclonedx` SBOM generation                                 | Cluster B retro       |
| 1000-event WS soak / slow-subscriber stress                       | Cluster B retro       |
| Mutation tests against bus + routes                               | Cluster B retro       |

### To Cluster D (FSM)

| What                                                  | Source           |
|-------------------------------------------------------|------------------|
| Real embedding generation pipeline                    | Cluster C retro  |
| Persistent event log + replay                         | Cluster C retro  |
| Resumable WS subscriptions / reconnection tokens      | Cluster B retro  |
| MCP `prompts/list` + `prompts/get`                    | Cluster B retro  |
| Per-model embedding tables / dimension variations     | Cluster C retro  |
| Faceted search (author / channel / kind filters)      | Cluster C retro  |

### To Cluster E (artifacts)

| What                                                   | Source           |
|--------------------------------------------------------|------------------|
| S3-compatible artifact backend                         | Cluster A retro  |
| Rich artifact taxonomy (full kind list per scope §46)  | Cluster A retro  |
| Streaming put/get for large payloads                   | Cluster A retro  |
| Artifact-kind-aware put helpers                        | Cluster A retro  |

### To Cluster F (auth)

| What                                              | Source           |
|---------------------------------------------------|------------------|
| HTTP / WS / MCP authentication                    | Cluster B retro  |
| Tokens, capabilities, ACLs                        | Roadmap          |
| Multi-tenant workspaces beyond a single org       | Architecture     |
| SQLite vector support via `sqlite-vec`            | Cluster C retro  |

### To Cluster G (A2A)

| What                                          | Source       |
|-----------------------------------------------|--------------|
| Agent-to-Agent transport over the event bus   | Roadmap      |

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

- **`v0.2.0` tag not yet cut.** PR #35 (retro) and PR #36 (release
  fix) are merged on `main`. The next step is:

  ```sh
  cd ~/bg/maidan
  git checkout main
  git pull --ff-only
  git tag -a v0.2.0 -m "Cluster C: search + indexing."
  git push origin v0.2.0
  ```

  This fires `release.yml` which should produce the GitHub Release +
  binaries + multi-arch ghcr.io images for both `maidan-server` and
  `maidan-postgres`.

- **The handoff doc PR itself** (this PR) is in flight; if you are
  reading this, it has merged.

- **Cluster D issues and plan doc** have not been written. The
  Cluster C retro names the priorities; "Cluster D backlog" above
  is the closest thing to a plan. The next agent should write
  `docs/Clusters/Cluster D.md` and the issues before opening D.1.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
  Items leave the list when the underlying issue is fixed.
- The per-cluster sections enumerate items the original PR scoped
  out. Items move from "deferred to" tables into their respective
  cluster's plan when work starts.
- A retro PR is the only legitimate moment to add items here. If
  you spot a deferred item that isn't listed, the previous retro
  missed it — open a follow-up PR that updates this file.
