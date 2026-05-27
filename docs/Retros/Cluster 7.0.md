# Cluster 7.0 retro — Bus pointer delivery

> Closing wave for Cluster 7.0 · target tag `v7.0.0`.

Cluster 7.0 made Postgres `LISTEN`/`NOTIFY` pointer-shaped: the wire carries
`log_id` (and optional `workspace_id`), and the listener hydrates the authoritative
row from `maidan_events`. Large envelopes no longer fail publish solely because
they exceed the legacy NOTIFY JSON cap.

## What shipped

- **PR #161** — Cluster kickoff plan ([[Clusters/Cluster 7.0]]).
- **PR #162** — Implementation bundle (7.0.1–7.0.4):
  - `Store::get_stored_event(log_id)` on Postgres and SQLite.
  - `PostgresBus` publishes `log_id_v1` pointer NOTIFY when `log_id > 0`;
    listener hydrates via `maidan_events`; `BusError::HydrateNotFound` /
    `HydrateFailed` on missing or corrupt rows.
  - Legacy full-envelope NOTIFY retained for `log_id == 0` (synthetic / tests).
  - Integration tests: pointer round-trip, large persisted event, synthetic still
    hits `PayloadTooLarge`.
  - [[Decisions]], [[Architecture]], [[Production]] updated for pointer-default semantics.

## What was deferred

| To         | What                                              | Why                                      |
|------------|---------------------------------------------------|------------------------------------------|
| Post-7.0   | At-most-once / outbox bus semantics               | Pointer path only; delivery guarantees unchanged. |
| Post-7.0   | Coverage floor toward 11%+                        | Separate measured CI wave.               |
| Post-7.0   | Per-model embedding tables / SQLite semantic      | Search-scope work.                       |
| Cluster B  | SSE for MCP `resources/subscribe`                 | Long-standing deferral.                  |
| Post-7.0   | `maidan_bus_notify_hydrate_total` counter           | Optional metric skipped when cheap path unclear. |

## Surprises

- Bundling store + bus + tests + docs in one PR stayed reviewable because the
  pointer contract is narrow and all call sites share the same publish order
  (`append_event` then `bus.publish`).
- Keeping synthetic `log_id == 0` on the legacy path preserved existing bus unit
  tests without inventing fake log rows.
- Testcontainers rewrite in `postgres_bus.rs` doubled as documentation for the
  new hydrate path.

## Decisions

- **Pointer default on Postgres** — NOTIFY is a wake-up + id; `maidan_events` is
  authoritative (reverses the pre-D “full JSON on NOTIFY” default).
- **InMemoryBus unchanged** — dev/test stays in-process full envelope; no hydrate.
- **Hydrate failures are drop + warn** — subscribers still recover via event-log
  replay and 6.0 metrics; no blocking retry loop on bad NOTIFY payloads.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| `Store::get_stored_event` by `log_id`                   | `v7.0.0`           |
| Postgres NOTIFY `log_id_v1` pointer + hydrate           | `v7.0.0`           |
| Large event publish on Postgres (beyond NOTIFY JSON cap) | `v7.0.0`          |
| Bus pointer delivery ops notes                          | `v7.0.0`           |

## Risks identified + mitigated

- **Large events fail Postgres publish** — pointer NOTIFY stays under cap; body
  lives in `maidan_events`.
- **NOTIFY payload bloat** — wire carries ids, not full envelopes.

## Risks identified + still open

- **At-most-once delivery** — NOTIFY remains fire-and-forget; replay paths unchanged.
- **Hydrate race / missing row** — rare if publish order holds; dropped notify +
  subscriber replay is the recovery story.
- **Coverage depth** — floor still 10.0%.
- **SQLite semantic search** — still unsupported.

## Forward look

Next wave is open planning: coverage uplift toward 11%+, or further reliability
work (outbox / stronger delivery semantics). See [[Open Work]].

## Acknowledgements

Solo cluster. Kickoff #161, implementation #162, this retro.
