# Cluster 234.0 retro — tasks get an answer slot

> Tag **`v234.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 18 — opens Arc F.

## What shipped

- The `maidan_thread_results` table + `ThreadResult` model + `set`/`get` store
  methods (both backends). Where a task's structured output lives — storage only, no
  routes or event yet.

## Surprises / decisions

- **Foundation-first, a fifth time — and it's muscle memory now.** DAG (217),
  scheduler (226), skills (230) all opened with a table + store and zero wiring;
  Arc F does the same. The five-place migration checklist (2 `.sql`, 2 const +
  apply, model, both `mod.rs`) went in without a stumble, and the IDE's
  missing-trait-items diagnostic flagged the impls the instant the trait grew.
- **One result, upserted — not a log.** The tempting richer design is a versioned
  result history. But a task has *an* answer; if it's revised, the latest is what a
  requester wants, and the event stream (235) will announce each change. `thread_id`
  PK + `ON CONFLICT DO UPDATE` is the whole thing. Versioning can layer on later if a
  real need appears; it isn't one now.
- **JSON, the same two-backend way as message content.** `result` is
  `serde_json::Value` — JSONB on Postgres (bind the value directly), TEXT on SQLite
  (serialize on write, parse on read, so `row_to_result` is fallible there and
  infallible on pg). Reusing the Cluster-173 pattern meant no new decisions.
- **This is the *what* to 222's *when*.** `ThreadReady` (222) already fires when a
  dependency finishes; a result gives the parent the payload to read. The two
  compose into the classic "spawn sub-tasks, wait, aggregate their outputs" shape —
  which 236's `wait_for_result` + dependency-results aggregate will complete.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_thread_results` + `ThreadResult` + set/get store methods | `migrations/*`, `models.rs`, `store/*/thread_results.rs` |

## Risks identified + still open

- None — a new table off every existing path.

## Forward look

Arc F builds out: **235** REST (`PUT`/`GET /threads/:id/result`) + a `ThreadResultSet`
event on set (observable like `ThreadReady`), then **236** MCP set/get + a
`wait_for_result` long-poll (the coordination wait, the `wait_for_ready` shape) + a
"read my dependencies' results" aggregate for parent tasks — which **closes Program
B**. Then Programs C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 233.0]].
