# Cluster 373 retro — Wave 2 #21: attachable labeled memory as room objects (H11)

Wave 2 #21 gives the room a new kind of shared object: a **memory block** —
a Letta-shaped `{label, description, limit, read_only, value}` unit of mutable,
labeled memory that lives in a workspace and can be **attached** to a thread.
Blocks are how a parent thread watches a child's result block **without a nested
runtime**: they share the block, and the child's write is the parent's read.
Deliberately **not a transcript** (`value` is replaced whole, last-writer-wins)
and **not RAG** (addressed by `label`, no embedding).

## What shipped

- **373.1 (#752) — the store foundation.** `maidan_memory_blocks` (pg 0078 /
  sqlite 0077, `UNIQUE(workspace_id, label)`) + `maidan_thread_memory_blocks`
  (attachments) + `MemoryBlock`/`NewMemoryBlock` + `MemoryBlockId` + pure
  `fits_char_limit` / `is_valid_block_label`. `MemoryBlockStore` (both backends):
  create (concurrent-safe on label — `ON CONFLICT DO NOTHING` + existing-row
  fallback), get/get_by_label/list, `set_value` (full rewrite, refuses read-only /
  over-limit), delete, attach/detach (idempotent), list-for-thread. Reads route
  via `read_pool()`. Zero-blast-radius.
- **373.2 (#753) — REST.** CRUD under `/workspaces/:wid/memory-blocks[/:id]` +
  thread attach/detach/list under `/threads/:id/memory-blocks[/:block_id]`.
  Reuses `workspace:read`/`write` (memory blocks are workspace content — the
  recipe-370 precedent; no new-cap deny-matrix ripple). read-only/over-limit →
  400; cross-tenant → 404 (no existence oracle). Full new-route preflight.
- **373.3 (#754) — MCP.** create/get/list/set/attach/detach/list-thread, blocks
  addressed by **label** within the caller's workspace (Letta-style). 5-place
  wiring + both sorted contracts.
- **373.4 (#755) — the reactive watch.** A new `MemoryBlockUpdated` event on
  set-value (REST + MCP, best-effort) + the MCP `wait_for_memory_block` long-poll
  — so a parent wakes on a child's write instead of polling. Full 11-site
  EventKind drill; non-federatable (a locally-derived signal over local shared
  state, like `ThreadResultSet`).
- **373.5 — this retro + the doc-close (with an audit fold, below).**

## Decisions

- **Label is the key, not the id.** A block's within-workspace `label` is its
  stable handle; the MCP tools and the watch address blocks by label, so a parent
  can `wait_for_memory_block("child.result")` before the child has even created
  it. `UNIQUE(workspace_id, label)` makes create-if-absent race-safe.
- **Full rewrite, last-writer-wins — not a transcript, not RAG.** `set_value`
  replaces the whole value; there's no append log and no embedding. A block is
  mutable shared state, the anti-pattern to both a message thread and a vector
  corpus. `read_only` freezes a block; `char_limit` (Letta's `limit`) bounds it.
- **The watch is a "go fetch" pointer.** `MemoryBlockUpdated` carries
  `{workspace_id, block_id, label, updated_by}` — never the (possibly large)
  value — so a waiter reacts and reads via `get_memory_block`. Non-federatable
  (the third such derived signal, after `ThreadReady`/`ThreadResultSet`).
- **Workspace-content caps, not a new capability.** Blocks reuse
  `workspace:read`/`write` (the recipe-370 call), avoiding churn in the two
  exhaustive deny-caps matrices; attach/detach add thread access on top.

## Surprises

- **The `wait_for_memory_block` filter can't pin the block.** `EventFilter` keys
  on workspace/channel/thread/member/kinds — not `block_id` — so the wait
  subscribes `kinds=memory_block_updated` scoped to the workspace and matches the
  requested block by **label** as events arrive (like `wait_for_ready` filtering
  by `can_access_thread`).
- **A `tokio::join!` needs `let`-bound args.** An inline `&json!(...)` in the
  join's first future is a temporary dropped at the statement's end while the
  future still borrows it (E0716) — bind the args first, as `wait_for_ready`'s
  test does.

## Test evidence

- Store: `memory_blocks` (both backends) — CRUD, concurrent-safe create, full
  rewrite, read-only + over-limit refusal, attach/detach, thread-list, NotFound.
- Types: `fits_char_limit`/`is_valid_block_label` units; the EventKind
  round-trip + federatable tripwires + the `event-kinds` contract.
- Server: `memory_block_rest_e2e` (auth-enabled + minted token) + `openapi_e2e`
  bijection + `http_capability_matrix_e2e` + `federation_e2e` (remap arm).
- MCP: `memory_block_tools_create_read_rewrite_and_attach` +
  `memory_block_wait_wakes_on_update` (`tokio::join` set→wake, + a different-block
  non-wake) + both contract-sync tests + `mcp_capability_matrix_e2e`.

## Audit fold (2026-09-10 world-class audit) — honesty first

A code-level audit reviewed the MCP write path against REST. Verified against the
live code and folded into Open Work under existing IDs (no new Wave number):

- **P1.1 was over-claimed.** The MCP write-path-parity work (Clusters 333–334)
  migrated the **message + social** tools (`edit_message`, `post_message`
  mentions, votes/reactions/pins/references) to the atomic `*_with_event` path,
  but the **assignment path was never migrated**: MCP `assign_thread` /
  `claim_thread` / `unassign_thread` / `claim_next_thread` still call the
  **non-event** store methods + a separate `publish_event` (a non-atomic
  dual-write), and MCP `claim_next` uses `claim_next_thread` (not
  `…_with_event`), so a reclaim of an expired lease does **not** emit
  `ClaimExpired`. On the agent-primary surface this is a real gap → tracked as
  **P1.1c** and fixed next (Cluster 374).
- **`StoreError::Conflict` → `McpError::Internal`** (a client conflict rendered
  as an internal error) → tracked, fixed in 374.
- **No MCP `transition_thread`** twin of the REST FSM transition → tracked (P1;
  confirm whether the omission is a deliberate land-gate or a gap).

## Forward look

**Wave 2 #21 is complete.** Deferred follow-ups: attach blocks to channels /
members (threads only today); block templates / per-block ACLs beyond
`read_only`; surfacing blocks on the `/ui`; a `MemoryBlockUpdated` hook in the
notification router. **Next: Cluster 374 = the P1.1c P0** (MCP assignment
dual-write), then **Wave 2 #22** (required reviewers).

## Acknowledgements

Four impl PRs (#752 store → #753 REST → #754 MCP → #755 event + watch) plus this
retro, on the foundation-then-wire + new-route-preflight + EventKind-drill +
capability-registry patterns.
