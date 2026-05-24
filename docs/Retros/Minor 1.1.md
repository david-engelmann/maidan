# Minor 1.1 retro — Delivery reliability

> Closing wave for optional minor **`v1.1.0`** · [[Post-1.0]] ladder 1.1.1–1.1.5.

After `v1.0.0`, the highest-impact standing risks were operator
visibility on the Postgres bus, client recovery after NOTIFY gaps, and
federation pull surviving restarts. This minor ships those fixes without
breaking the public API.

## What shipped

| PR   | Scope |
|------|-------|
| #107 | Postgres `LISTEN` listener health on `/health/ready` (`bus` subsystem). |
| #108 | `BusItem::Lagged` + WS/MCP `replay_hint` frames with `log_id`. |
| #109 | Subscribe/MCP `after_id` replay (≤500 events) + watermark dedupe. |
| #110 | Migration 0010: encrypt peer outbound secrets (`FEDERATION_ENCRYPTION_KEY`). |
| #111 | Migration 0011: `remote_workspace_id`; pull-path compose smoke. |

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Track T   | `cargo-llvm-cov` coverage gate                    | Optional T.3.                            |
| Track W   | OpenAPI / docs site                               | Documentation track.                     |
| Track V   | GDPR erasure, Sigstore releases                   | Security track.                          |
| `v1.2.0`  | Real embeddings + faceted search                  | [[Post-1.0]] minor 1.2 ladder.           |

## Surprises

- **Pull smoke needs Docker DNS** — peer `base_url` for the worker must be `http://maidan-a:8080`, not host `localhost`.
- **Hydrate vs lazy decrypt** — startup hydration warms cache; poll still decrypts on first tick if cache empty.

## Decisions

- **`remote_workspace_id`** — `workspace_id` is the local ingest target; `remote_workspace_id` is the workspace polled on the peer's `base_url` (defaults to local id when omitted).
- **Encryption key is operator-owned** — backup `FEDERATION_ENCRYPTION_KEY` with the database; rotation requires re-creating peers.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Postgres bus listener health on readiness               | `v1.1.0`           |
| WS/MCP `replay_hint` on subscriber lag                    | `v1.1.0`           |
| Resumable subscribe via `after_id` / `Last-Event-Id`    | `v1.1.0`           |
| Encrypted federation peer outbound secrets at rest      | `v1.1.0`           |
| Federation pull-path compose CI smoke                   | `v1.1.0`           |

## Risks identified + still open

- **At-most-once NOTIFY** — clients must still reconnect with `after_id`; hints are advisory only.
- **Bootstrap routes** remain unauthenticated.
- **Symmetric peer registration** — pull to an auth-enabled remote still requires a matching peer row on the upstream.

## Forward look

Optional **`v1.2.0`** (search + embeddings) per [[Post-1.0]], or continue Track T/W.

## Acknowledgements

Solo minor. Five PRs, one retro, tag `v1.1.0`.
