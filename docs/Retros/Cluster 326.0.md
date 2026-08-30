# Cluster 326.0 retro — as-of context replay

> Tag **`v326.0.0`**. Phase XXIV (post-gate hardening). **Cluster 8 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The net-new context lane: `GET /threads/:id/context?as_of=<event_id>` (+ the MCP
`get_thread_context` `as_of` arg) reconstructs a thread **as it stood at that event-log
id** — deterministic over the immutable log, no fresh semantic search. Serves audit ("what
context did the agent see when it produced result X") and re-ask-from-before-a-tangent.

- **`Store::list_thread_events_through(thread_id, through_id)`** (both backends) — the
  thread's events with `id <= through_id`, the immutable substrate.
- **`maidan_types::reconstruct_messages_through(events)`** — folds `MessagePosted` /
  `MessageEdited` (both carry the full `Message`) and `MessageTombstoned` (removes it) into
  the as-of message set, preserving posted order. **Shared** by the REST assembler and the
  MCP builder (one fold, no duplication).
- **REST assembler `build_thread_context_as_of`** and **MCP `get_thread_context_as_of`** —
  message set from the event fold; additive components (edits, references, transitions,
  artifacts) cut by the anchor event's time; the as-of FSM state is the last transition
  ≤ cutoff; the glossary (current vocabulary, not thread history) is omitted.

## Surprises / decisions

- **Events carry the whole `Message`.** Because `MessagePosted`/`MessageEdited` embed the
  full message, a since-edited message shows its **as-of body** and a since-tombstoned
  message **reappears** — impossible from current rows alone (tombstone zeroes the body).
  This is why reconstruction goes through the log, not a time-filter on `maidan_messages`.
  "Deterministic over immutable data" = the event log, literally.
- **Hybrid cutoff.** Messages are cut by **log position** (exact); the additive-only
  components (edits/refs/transitions/artifacts are append-only immutable rows) are cut by
  the anchor's **time** — simpler than event-sourcing each, and correct because they never
  mutate in place. Documented as a v1 boundary (sub-second cross-component ordering).
- **Any event id is a valid cutoff.** `as_of` need not belong to the thread — it is a global
  log position; a missing id is `404` (`get_stored_event` → `NotFound`).
- **Thread-scoped only.** Workspace-context as-of is out of scope for v1 (the plan specified
  the thread endpoint); the workspace route passes `as_of: None`.

## Test evidence

`as_of_replay_e2e` (REST: as-of shows the original body pre-edit, the edited body post-edit,
a since-tombstoned message present before its tombstone and gone at it, live pack unchanged,
unknown id → `404`) — this exercises the event-log fold end-to-end;
`context::tests::as_of_replay_shows_the_message_body_at_that_point` (MCP twin);
`context_query_count_e2e` + `thread_context_e2e` + `glossary_context_e2e` + `openapi_e2e`
unchanged/green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean; mdbook
linkcheck green.

## Forward look

Next in the arc: **seed-from-message** (the write side of re-ask — `POST
/messages/{id}/seed`, a `seeded_from` typed edge, no bespoke table; `prefix` mode delegates
to this as-of replay) → immutable context snapshot artifact → flow template.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
