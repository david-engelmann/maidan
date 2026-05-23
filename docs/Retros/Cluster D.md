# Cluster D retro — FSM-driven thread lifecycle

> Closing wave for Cluster D · target tag `v0.3.0`.

Cluster C made the workspace searchable. Cluster D makes thread
progress explicit: a typed FSM, an append-only transition log, nested
threads with hierarchical rules, deterministic embeddings, a persistent
event log for replay, and MCP prompts for agent workflows.

## What shipped

- **PR #48** — `feat(maidan-store): schema 0004 thread transitions + in_review` — `maidan_thread_transitions` table and `in_review` state on both dialects.
- **PR #49** — `feat(maidan-fsm): typed thread lifecycle state machine` — `maidan-fsm` transition graph (`start_review` / `close` / `archive`).
- **PR #50** — `feat(maidan-server): transition API, store, ThreadStateChanged event` — `POST /threads/:id`, 409 on illegal edges, bus event.
- **PR #51** — `feat(maidan-fsm): hierarchical state machine for nested threads` — `parent_thread_id` + HSM child/parent ordering.
- **PR #52** — `feat(maidan-search): hash-v1 embedding generation in indexer` — `EmbeddingHandler` on Postgres.
- **PR #53** — `feat(maidan-store): persistent event log + replay` — `maidan_events` + `GET /workspaces/:wid/events`.
- **PR #54** — `feat(maidan-mcp): prompts/list + prompts/get` — `thread_workflow` prompt.

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Cluster E | S3 artifact backend                               | Artifact cluster scope.                  |
| Cluster T | Indexer lag on `/health`                          | Observability cluster.                   |
| Cluster T | NOTIFY id-pointer as default bus payload          | Full payload still works for small events. |
| Cluster F | Thread reopen transitions                         | Pre-1.0 scope cut.                       |
| Post-1.0  | Real ML embedding model                           | `hash-v1` proves the indexer path.       |

## Surprises

- **PR #47 closed when retargeting base** — reopened as #48; retarget stacked PRs carefully.
- **SQLite `last_insert_rowid()` via pool** — unreliable for event log reads; `INSERT … RETURNING` fixed it.
- **`main.rs` move errors** — `AppState::new` consumed `store`/`search` before `EmbeddingHandler`; build indexer handler first.

## Decisions

- **HSM rule: child rank ≤ parent rank** — child cannot be `in_review` while parent is `open`. Stays.
- **`hash-v1` embeddings** — deterministic SHA-256 expansion to 1024-d; no model server in v0.3.0.
- **Event log before bus publish** — append failures are logged; bus remains best-effort.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Thread FSM (`open` → `in_review` → `closed` → `archived`) | `v0.3.0`           |
| `POST /threads/:id` state transitions                     | `v0.3.0`           |
| `ThreadStateChanged` bus event                            | `v0.3.0`           |
| Nested threads (`parent_thread_id`) + HSM                 | `v0.3.0`           |
| `hash-v1` indexer embeddings (Postgres)                   | `v0.3.0`           |
| Persistent `maidan_events` log + replay HTTP API          | `v0.3.0`           |
| MCP `prompts/list` + `prompts/get`                      | `v0.3.0`           |

## Risks identified + mitigated

- **Illegal transitions corrupting state** — FSM enforced in `maidan-fsm` + store transaction; HTTP returns 409.
- **Child thread outrunning parent** — HSM check on every child transition.

## Risks identified + still open

- **At-most-once bus delivery** — mitigated partially by event log replay API; subscribers still need to call replay on gap.
- **SQLite semantic search** — still unsupported; Postgres required for vectors.

## Forward look

Cluster E is artifact substrate (S3, rich taxonomy). Verify `v0.3.0` GitHub Release artifacts before tagging.

## Acknowledgements

Solo cluster. The FSM/HSM split (`maidan-fsm` pure, store/server wired) kept PRs reviewable.
