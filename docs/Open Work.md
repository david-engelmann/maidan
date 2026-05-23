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
  no automatic WS backfill yet. → Cluster T / future subscriber work.
- **Bootstrap routes are unauthenticated.** `POST /workspaces` and
  `POST …/members` have no Bearer gate; production must seed with
  `AUTH_DISABLED` then mint tokens before enabling auth.
- **No indexer lag metric on `/health`.** A stuck indexer is
  invisible to operators. → Cluster T.
- **`v0.1.0` GitHub Release didn't auto-create.** Cleanup PR landed
  (#36 → `macos-13` for x86_64 darwin). Verify `v0.3.0` tag triggers
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

- **Cluster D** complete (`v0.3.0`). Plan:
  [`docs/Clusters/Cluster D.md`](Clusters/Cluster%20D.md). Retro:
  [`docs/Retros/Cluster D.md`](Retros/Cluster%20D.md). Tag `v0.3.0`
  after retro PR merges.
- **Cluster E** complete (`v0.4.0`). Retro:
  [`docs/Retros/Cluster E.md`](Retros/Cluster%20E.md).
- **Cluster F** complete (`v0.5.0`). Retro:
  [`docs/Retros/Cluster F.md`](Retros/Cluster%20F.md). Tag `v0.5.0` after
  retro PR merges.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
  Items leave the list when the underlying issue is fixed.
- The per-cluster sections enumerate items the original PR scoped
  out. Items move from "deferred to" tables into their respective
  cluster's plan when work starts.
- A retro PR is the only legitimate moment to add items here. If
  you spot a deferred item that isn't listed, the previous retro
  missed it — open a follow-up PR that updates this file.
