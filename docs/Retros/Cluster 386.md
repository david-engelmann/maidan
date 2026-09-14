# Cluster 386 retro — Wave 2 #27: a closed blocked-reason enum

Wave 2 #27 asked for a **blocked-reason enum** (`dag|gate|human|child|quota|unclaimable`)
so an orchestrator can park a thread from dispatch with a named why,
`claim_next` skips it, and `BlockedResolved` unblocks. This is **not**
Cluster 217/218 DAG-children-must-be-terminal (that skip stays derived
from the dependency graph) and **not** Cluster 363's unclaimable park
table (`unclaimable` here is one vocabulary value).

The enum is **closed** — unlike `result_kind`, which is a namespaced
string a producer publishes.

Four impl PRs (386.1–386.4) + this retro. Cluster 384 was already
claimed by P1.1d MCP `transition_thread` (#816/#823); Soundcheck took
385; run-lineage took 387. This stack is 386. This retro targets
`main` only (the 383 `base_ref_deleted` lesson). Children #820/#822/#825
were retargeted to `main` before any parent squash-delete.

## What shipped

- **386.1 (#817) — types + store.** Closed `BlockedReason` +
  `ThreadBlock`. `maidan_thread_blocks` (pg 0089 / sqlite 0088). Store
  CRUD: `set_thread_block` (upsert) / `clear_thread_block` /
  `get_thread_block` / `list_blocked_threads`. Zero blast: `claim_next`
  unchanged.
- **386.2 (#820) — claim_next skip.** All four SQL sites
  (`claim_next` / `claim_next_with_event`, both backends) skip a block
  row. Queue-depth `ready` matches; `blocked` is explicit **or**
  unfinished DAG deps. Occupancy `queued`/`blocked` follow. 218 and 363
  stay.
- **386.3 (#822) — `BlockedResolved`.** `EventKind` / `Event` +
  `clear_thread_block_with_event` (delete + append in one tx).
  Non-federatable (local dispatch decision). Full EventKind drill +
  `contracts/event-kinds.json`.
- **386.4 (#825) — REST / MCP / e2e.** `PUT`/`GET`/`DELETE
  /threads/:id/block` + `GET /channels/:cid/blocked`. MCP
  `set_thread_block` / `get_thread_block` / `clear_thread_block` /
  `list_blocked_threads`. Explicit `claim` 409 / InvalidParams.
  `DELETE` publishes `BlockedResolved`. HTTP + MCP e2e observe the bus.

## Decisions

- **Closed enum, not a namespaced string.** The six reasons are the
  product lock. An unknown reason is 400 / InvalidParams at the
  extractor.
- **Side table, presence = blocked.** Absence is unblocked. One row per
  thread (upsert). Clearing is the unblock.
- **`child` is an orchestrator-named wait**, not "every DAG child must
  be terminal". Cluster 218 stays the derived readiness skip.
- **363 unclaimable stays.** The park table is unchanged. `unclaimable`
  in the enum is shared vocabulary, not a replacement.
- **Queue-depth `blocked` folds both causes.** `open = ready + assigned
  + blocked + unclaimable` stays exact. The *reason* is on
  `get_thread_block`.
- **`BlockedResolved` is non-federatable.** Same class as `ThreadReady`
  — a locally-derived signal a peer must not inject.
- **Did not add `wait_for_blocked_resolved`.** The e2e bus subscribe is
  enough to observe. A long-poll can layer later like `wait_for_ready`.

## Surprises

- **Cluster-number collision mid-stack.** 384 was claimed by P1.1d
  after 386.1 titled itself 384.1. Renumber in titles/comments; do not
  rewrite the impl commit history.
- **A stacked PR targeting a deleted base is closed, not retargeted.**
  383 lost #810/#812 that way. Children were retargeted to `main`
  before any squash-delete.
- **Occupancy's last SELECT column cannot have a trailing comma.**
  SQLite reports `near "FROM": syntax error`. Queue-depth hid the same
  edit because `unclaimable_count` follows `blocked`.
- **`set_by` / `resolved_by` are NOT-NULL FKs.** Bypass (nil member)
  FK-fails. e2e and the MCP test mint a real token / session, same as
  363.
- **Soundcheck took the next migration slot while 386.1 was open.**
  385.1 landed pg 0088 / sqlite 0087. Blocks stayed **pg 0089 /
  sqlite 0088** — rebase must bump the version, not overwrite
  Soundcheck. #817, #820, #822, and #825 then squash-merged onto
  `main`. This retro rebased onto that tip after #829.

## Test evidence

- Types: `blocked_reason` — every variant round-trips; unknown does
  not parse. `EventKind` ALL / federatable / contract include
  `blocked_resolved`.
- Store, both backends (`thread_blocks`): set/get/list/upsert; older
  `child`-blocked skipped by `claim_next`; DAG-blocked sibling still
  skipped by 218; clear restores claimability; `BlockedResolved`
  payload carries reason + resolver. Postgres testcontainers skip
  without Docker.
- Server: `thread_block_e2e` (auth-enabled, minted token) — PUT gate /
  GET / list / unknown 400 / claim 409 / claim_next skips / DELETE 204
  + bus `BlockedResolved` / second DELETE 404 / then claimable.
  OpenAPI bijection + capability matrix green.
- MCP: `block_tools_park_a_thread_from_dispatch` — same loop; clear
  publishes `BlockedResolved` on the attached bus. Catalog +
  capability-map contracts green.
- Clippy `-D warnings` on types/store/mcp/server.

## Forward look

**Cluster 386 is complete. Row #27 is closed.**

Do not start Wave 3/4 from this retro. **#25 is already shipped**
(Cluster 383 composition + Cluster 385 pointer). #26 capability
ticket stays open. #28 follow-occupancy and the manager digest remain;
the lineage half already shipped as Cluster 387 (#818/#824/#828/#829).
This retro strikes **#27 only**.

Deferred: `wait_for_blocked_resolved`; a `/ui` block toggle; folding
363's park into this table (declined — two mechanisms, one vocabulary
value).

## Acknowledgements

Four impl PRs (#817 store → #820 skip → #822 event → #825 REST/MCP/e2e)
+ this retro. Cluster 384/385/387 agents kept their numbers; we moved.
