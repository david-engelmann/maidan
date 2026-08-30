# Cluster 325.0 retro — agent conventions (decisions, supersession, acks)

> Tag **`v325.0.0`**. Phase XXIV (post-gate hardening). **Cluster 7 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The "near-zero-code conventions" half of the arc's confidence-and-conventions item —
codified as documentation with a **convention-proving e2e**, and **zero new server code**.
Maidan stays a room, not a brain: the server already has every primitive; agents agree on
how to use them.

- **`docs/Integration.md` → "Agent conventions"** documents three patterns over existing
  primitives:
  - **Decision records** — an ADR-shaped `thread_result` JSON
    (`kind/status/context/decision/consequences/alternatives`; `status ∈
    proposed/accepted/rejected/superseded`) over Cluster 235 `thread_results`.
  - **Supersession** — a typed `supersedes` reference edge (Cluster 319) from the new
    decision's thread to the old, plus flipping the old record's `status`; `GET
    /references?dst_kind=thread&dst_id=…&relation=supersedes` answers "what replaced this?".
  - **Grounding acks** — an `ack` vote (Cluster 324 `confidence` optional) that grounds the
    message *as of the vote's `created_at`*, so it is **stale** once the message is edited
    later (compare `created_at` to `message_edits[].edited_at`, both in the context pack).
- **`decision_convention_e2e`** exercises the whole trio over the real HTTP API — set a
  decision record, supersede it, query the reverse edge, ack a message, edit it, and detect
  the ack is stale — proving the room supports the pattern with no server change.

## Surprises / decisions

- **Docs + test, no code — deliberately.** "A room, not a brain" is a locked anti-goal;
  putting a typed `DecisionRecord` in the server or validating the shape server-side would
  make Maidan opinionated about decision structure. The convention lives in the client's
  head; the server stays a generic `thread_results` store. The e2e is the teeth — it proves
  expressibility rather than adding enforcement.
- **Ack staleness is time-pinned, not id-pinned.** Deriving staleness from
  `created_at` vs `edited_at` (both already exposed) needs no new column; the test uses a
  1.1s gap so the assertion is robust even at SQLite's second-granularity `datetime('now')`.
- **`thread_result` confidence folds in here.** `thread_results` already stores arbitrary
  JSON, so a decision's confidence is just a field in the record — no schema change, unlike
  the vote `confidence` column (324).

## Test evidence

`decision_convention_e2e` (auth-enabled + minted token: decision record set/get, `supersedes`
edge + reverse relation-filtered query, ack → edit → stale-detection) green;
`--all-targets` clippy + fmt + mdbook linkcheck clean. No lib/schema change, so the strict
lib clippy and bootstrap-strip are unaffected.

## Forward look

The confidence-and-conventions item (324 + 325) is complete. Next, the net-new context
lane: **as-of context replay** (`GET /threads/:id/context?as_of=<event_id>` + MCP twin —
deterministic over the immutable log, no fresh semantic search) → seed-from-message →
immutable context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
