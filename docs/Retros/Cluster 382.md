# Cluster 382 retro — Wave 2 #24 pack half: claimer pack includes accepted decisions

Wave 2 #24 asked for two things: the next `claim_next` claimer must see in-channel
**accepted/closed decisions**, and `result_kind` must be a search facet. This
cluster is **only the pack half**. The facet is **Cluster 381** and stays open
(namespaced string, not a closed enum — the 2026-09-12 correction).

`claim_next` itself still returns `Option<Thread>`. The pack is
`get_thread_context` (REST + MCP). Full result payloads stay on
`GET /threads/:id/result`; the pack carries token-lean teasers.

Three impl PRs (382.1–382.3) + this retro.

## What shipped

- **382.1 (#792) — store.** `Store::list_channel_closed_results(channel_id,
  exclude_thread_id, limit)` (both backends): JOIN `maidan_thread_results` ×
  `maidan_threads`, `state IN ('closed','archived')`, non-tombstoned, newest
  `produced_at` first. `limit` clamped `1..=50`. The store is dumb — no JSON /
  `result_kind` interpretation. `ChannelClosedResult` in `maidan-types`.
- **382.2 (#797) — REST pack.** `AcceptedDecision` teasers on the live
  `ThreadContext` (`thread_id`, `title`, `state`, `produced_by`, `produced_at`,
  optional `result_kind` / `status` / `summary`). Default **on**;
  `include_accepted_decisions=false` drops them. Cap 10
  (`ACCEPTED_DECISIONS_LIMIT`). Withheld on `__dm__`, as-of packs, and
  workspace-nested packs. OpenAPI `AcceptedDecision` + Integration.md. The
  assembler (`assemble_accepted_decisions`) is the smart layer: a
  `pi.waiter.result/1` envelope is included **only** when `status == reviewed`;
  opaque JSON on a terminal thread is accepted (the Cluster-359 "closed + has a
  result" model); a free-form string `result_kind` is copied through without
  requiring the waiter schema. Full `rendered` / findings are not inlined
  (240-byte UTF-8 excerpt).
- **382.3 (#796) — MCP twin.** `get_thread_context` /
  `snapshot_thread_context` carry the same field (MCP has its **own** assembler,
  not `build_thread_context`). Catalog `include_accepted_decisions` (default
  true). Workspace nested packs still withhold. `maidan-fsm` added as an MCP
  **dev-dep** so the pack test can close threads.
- **382.4 — this retro + the doc-close.** Strike the pack half of Open Work
  #24; leave Cluster 381 as the remaining facet half.

## Decisions

- **The pack is the claimer's read, not a `claim_next` payload change.**
  Returning teasers from `claim_next` would have broken every client of
  `Option<Thread>` (REST, MCP, SDKs). The claimer already fetches context; that
  is where orientation belongs.
- **`result_kind` is a namespaced string, not a closed enum.** The original
  Open Work row assumed `decision|plan|merge_authorized`. A live pi result
  publishes `result_kind = "pi.review.result/1"` inside `schema =
  "pi.waiter.result/1"`. An enum would need editing every time a waiter
  product ships a new kind. The store does not interpret the string; the pack
  copies it when present.
- **Waiter envelopes are accepted only when `reviewed`.** In-flight /
  failed producer states are not decisions. Opaque JSON on a closed thread
  has no such discriminator, so "closed + has a result" stands (Cluster 359).
- **Do not inline `rendered`.** The teaser is orientation; the payload is a
  fetch. A busy channel of full GitHub-sized `rendered` bodies would blow the
  token budget the pack exists to protect.
- **Default on, small cap, explicit opt-out.** A claimer who does not know
  the flag still sees the channel's decisions. Ten teasers is orientation; the
  store's 50 clamp is a safety rail, not the pack default.
- **Skip `__dm__`.** The shared DM channel is not a decision room — two 1:1
  conversations would leak into each other's packs. As-of packs are a
  historical snapshot, not a live claimer view. Workspace nested packs would
  multiply the extra query across a page of threads.

## Surprises

- **MCP does not share `build_thread_context`.** The REST field does not
  appear on MCP packs until the MCP assembler is taught the same rules — the
  360.3 / 360.4 split again. 382.3 is not a catalog-only follow-up.
- **A stacked PR targeting a deleted base is closed, not retargeted.** 382.2
  originally stacked on 382.1's branch (#795). Squash-merge deleted that
  branch (`base_ref_deleted`) and GitHub closed #795. #797 is the
  replacement, rebased onto `main`. 382.3 (#796) was retargeted to `main`
  *before* #797's branch delete, then rebased onto the 382.2 squash. Same
  lesson as 379.4 / #790.
- **`maidan-fsm` is not a transitive import for MCP tests.** Closing threads
  in the pack test needed an explicit `maidan-mcp` dev-dep. Clippy/test would
  not have compiled the suite without it.
- **A fake `__dm__` channel GET 403s.** Cluster-180 DM-participant checks
  mean the e2e has to open two real 1:1 DMs (shared `__dm__`, distinct
  threads) rather than inserting a synthetic DM channel.

## Test evidence

- Store, both backends: `channel_closed_results` — closed + archived included,
  open / tombstoned / other-channel excluded, `exclude_thread_id` drops the
  claimer's own thread, `limit` clamps, newest `produced_at` first.
  `dialect_parity` + `backend_parity` + `concurrent_migrations` green.
- Types: pack unit tests — reviewed waiter included; non-reviewed waiter
  dropped; opaque JSON accepted; free-form `result_kind` copied without the
  waiter schema.
- Server: `thread_context_lists_in_channel_accepted_decisions` (auth-enabled)
  — live pack lists a reviewed in-channel close, drops a non-reviewed waiter,
  opt-out empties the field, DMs / as-of / workspace-nested withhold.
  OpenAPI `AcceptedDecision` registered.
- MCP: `thread_pack_lists_in_channel_accepted_decisions` (same policy on the
  MCP assembler). Catalog schema updated; no new tool (contract names
  unchanged).

## Forward look

**The pack half of Wave 2 #24 is complete.** A claimer who then calls
`get_thread_context` sees what this channel already decided.

**Remaining of row #24:** Cluster 381 — facet `result_kind` as a namespaced
string on search / list-results (open; this retro does not close it).

**Not this cluster:** Cluster 380 (inline per-finding PR comments) and
Cluster 379 (result delivery) are a different arc. P1.1d (`transition_thread`
MCP) was not taken.

Deferred (follow-ups): inlining teasers on the `claim_next` response itself
(declined — payload break); a `/ui` accepted-decisions strip on the Work tab;
pushing the deny-set / private-channel filter into the closed-results query
(the pack's thread-RBAC still filters the claimer's *own* thread; the list is
same-channel by construction).

## Acknowledgements

Three impl PRs (#792 store → #797 REST pack → #796 MCP) + this retro. **382.2's
first PR (#795) was closed by GitHub** when 382.1's branch was deleted under
it; #797 is the replacement on `main`. #796 stayed `base=main` through that
delete, then rebased onto the 382.2 squash.
