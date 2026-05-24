# Track T — Telemetry + performance

Cross-cutting track (no version tag). Raises operator visibility after
the `v1.0.0` cluster ladder. Request-id middleware shipped in Cluster H;
this track completes observability and related perf hygiene.

> **Goal:** Optional OTLP trace export, structured JSON logs, indexer
> visibility on `/health`, and documented env contract.
>
> **Not a cluster** — no retro tag; ship as numbered PRs on `main`.

## PRs

| #    | Title                                                          | Issue |
|------|----------------------------------------------------------------|-------|
| T.1  | `feat(maidan-observability): OTLP + JSON logs + request spans`  | #102  |
| T.2  | `feat(maidan-server): indexer heartbeat on /health/ready`      | #103  |
| T.3  | `chore(ci): cargo-llvm-cov coverage job` (optional)            | TBD   |

## Order

1. **T.1** — `maidan_observability::init` from env (`OTLP_ENDPOINT`,
   `OTLP_SERVICE_NAME`, `MAIDAN_LOG_FORMAT=json`); wire `maidan-server`
   `main`; HTTP spans carry `request_id`.
2. **T.2** — `indexer_last_event_at` on readiness JSON; `INDEXER_STALE_SECS`
   threshold marks indexer stale.
3. **T.3** — coverage upload when CI budget allows.

## Env contract

| Variable              | Default              | Purpose                          |
|-----------------------|----------------------|----------------------------------|
| `MAIDAN_LOG`          | `info,sqlx=warn`     | `tracing` filter (unchanged)     |
| `MAIDAN_LOG_FORMAT`   | `plain`              | `plain` or `json`                |
| `OTLP_ENDPOINT`       | unset                | gRPC collector, e.g. `:4317`      |
| `OTLP_SERVICE_NAME`   | `maidan-server`      | OTel resource service.name       |

## Exit criteria

- CI green.
- Server starts with and without `OTLP_ENDPOINT`.
- `/health/ready` includes indexer timestamp when indexer runs.
- `docs/Open Work.md` standing risks updated.

## Out of scope

- Prometheus metrics exporter (later T PR).
- Automatic WS replay on NOTIFY miss.
- Federation compose smoke (separate chore PR).
