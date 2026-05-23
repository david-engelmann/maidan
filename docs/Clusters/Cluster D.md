# Cluster D — FSM-driven thread lifecycle

After Cluster C made the workspace searchable, Cluster D makes thread
state explicit, auditable, and machine-enforceable. A typed transition
graph replaces ad-hoc `state` column updates; a persistent transition log
feeds replay and the event-log work deferred from B/C.

> **Goal:** Threads move through `open` → `in_review` → `closed` →
> `archived` via legal transitions only. Every transition is recorded
> with actor + timestamp. Subscribers see `thread_state_changed` on the
> bus. The persistent event log closes the at-most-once gap.
>
> **Target tag:** `v0.3.0`.

## PRs

| #       | Title                                                                 | Issue |
|---------|-----------------------------------------------------------------------|-------|
| D.1     | `feat(maidan-store): schema 0004 thread transitions + in_review`      | #38   |
| D.2     | `feat(maidan-fsm): typed thread lifecycle state machine`              | #39   |
| D.3     | `feat(maidan-server): transition API, store, ThreadStateChanged event` | #40  |
| D.4     | `feat(maidan-fsm): hierarchical state machine for nested threads`     | #41   |
| D.5     | `feat(maidan-search): real embedding generation in indexer`           | #42   |
| D.6     | `feat(maidan-store): persistent event log + replay`                     | #43   |
| D.7     | `feat(maidan-mcp): prompts/list + prompts/get`                        | #44   |
| D.retro | `docs(retro): Cluster D retrospective + v0.3.0 tag prep`              | #45   |

## Order

1. **D.1 first** — migration 0004 on both dialects: `maidan_thread_transitions`
   table + `in_review` in the `maidan_threads.state` check. Types and
   row parsers only; no transition API yet.
2. **D.2** — pure `maidan-fsm` crate: allowed edges, `InvalidTransition`
   errors, unit tests for the full matrix.
3. **D.3** — wire FSM to `Store` (append transition row, update thread
   state), HTTP route, and `ThreadStateChanged` on the bus. Illegal
   transitions return 409 via RFC 7807.
4. **D.4** — HSM: `parent_thread_id` schema + rules for nested threads
   (child state constrained by parent).
5. **D.5** — swap `LoggingHandler` for a real embedding `EventHandler`;
   respect thread state (e.g. skip archived).
6. **D.6** — `maidan_events` table, NOTIFY id-pointer path, replay for
   missed notifications (see [[Decisions]]).
7. **D.7** — MCP `prompts/list` + `prompts/get` per thread.
8. **D.retro** closes the cluster + cuts `v0.3.0`.

D.6 could theoretically land before D.4 if replay is urgent, but the
plan keeps the FSM surface stable before reshaping the bus.

## Transition graph (v0.3.0)

```text
        ┌──────────┐
        │   open   │
        └────┬─────┘
             │ start_review
             ▼
      ┌─────────────┐
      │  in_review  │
      └──────┬──────┘
             │ close
             ▼
        ┌─────────┐     archive      ┌───────────┐
        │ closed  │ ───────────────► │ archived  │
        └─────────┘                  └───────────┘
```

- `open` → `in_review` (`start_review`)
- `in_review` → `closed` (`close`)
- `closed` → `archived` (`archive`)
- No backward edges in v0.3.0 (reopen is a Cluster F+ candidate).

## Exit criteria

- CI green on `main`.
- Illegal transitions rejected at store + HTTP; legal ones append a
  `maidan_thread_transitions` row and publish `ThreadStateChanged`.
- Persistent event log: subscriber can recover events missed during a
  NOTIFY gap (integration test).
- Indexer generates embeddings on Postgres for `MessagePosted` when the
  thread is not `archived`.
- MCP prompts list/get round-trip in the MCP test harness.
- [[Retros/Cluster D]] merged.
- `v0.3.0` tagged and GitHub Release workflow produces all binaries +
  ghcr images (verify before retro merge).

## Risks

| Risk                                                                 | Mitigation                                                                 |
|----------------------------------------------------------------------|----------------------------------------------------------------------------|
| SQLite cannot alter `CHECK` in place                                 | D.1 recreates `maidan_threads` with `PRAGMA foreign_keys=OFF`.             |
| FSM + event log + HSM in one cluster is large                      | Strict PR scope; defer reopen / extra edges to Open Work.                |
| Embedding model load time at server boot                           | Lazy load on first `MessagePosted` in D.5; document in retro.            |
| NOTIFY payload vs event-log row size                               | D.6 switches default to id-pointer; full JSON for small events only.     |
| HSM without `parent_thread_id` in schema                           | D.4 adds column + migration before HSM rules.                              |
| `v0.2.0` release workflow incomplete                               | Confirm Release artifacts before cutting `v0.3.0`.                         |
