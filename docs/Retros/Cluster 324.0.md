# Cluster 324.0 retro — optional vote confidence

> Tag **`v324.0.0`**. Phase XXIV (post-gate hardening). **Cluster 6 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

An optional **confidence** weight on a vote, so a consumer can compute *weighted*
consensus instead of a flat tally — the first slice of the arc's "confidence + conventions"
item.

- **`maidan_votes.confidence`** (pg `0054` `DOUBLE PRECISION` / sqlite `0053` `REAL`),
  nullable. `Vote.confidence` / `NewVote.confidence` = `Option<f64>`
  (`skip_serializing_if = "Option::is_none"`, so a vote without one is byte-identical).
- **Range `0..=1` is an API-edge convention** (REST `400` / MCP `InvalidParams`), not a
  column CHECK — the store stays mechanism, the policy lives at the boundary.
- **Re-cast upserts the confidence.** The `(message_id, member_id, kind)` PK's
  `ON CONFLICT` moved from `DO NOTHING` to `DO UPDATE SET confidence = excluded.confidence`
  in both `cast` and `cast_with_event` (both backends) — a member updating their stated
  confidence is the natural gesture, count stays idempotent.
- Surfaced on REST `POST/GET /messages/:id/votes` (`CreateVote.confidence`) and the MCP
  `cast_vote` tool.

## Surprises / decisions

- **Contained ripple.** `Vote` is read in exactly two places (`{pg,sqlite}/votes.rs::list`)
  and there is no shared `row_to_vote`, so the Cluster-171 schema-column-ripple didn't
  bite — but I still ran the full both-backend roundtrip suites (memory
  `maidan-schema-column-ripple`) to be sure.
- **Event untouched.** `Event::VoteCast` still carries just `vote_kind`; putting confidence
  on the event would add a field to an existing variant (federation-remap + accessor +
  test ripple) for a stream-consumer nicety. Deferred — the stored vote + `list` is the
  consensus substrate this cluster needed.
- **DO UPDATE clears on a no-confidence re-cast** (latest cast wins, including back to
  `NULL`). Predictable and simple; documented in the store comment.

## Test evidence

Both-backend roundtrip suites now cast with confidence and assert the re-cast **updates** it
(`common::run_full_roundtrip`, pg + sqlite); `vote_confidence_e2e` (REST: cast/read/re-cast-
updates/no-confidence-omits-field/out-of-range-400); `event_log` (VoteCast atomic append)
green; MCP contract-sync + capability matrix + `openapi_e2e` bijection green. fmt + strict
clippy + `--all-targets` + bootstrap-strip clean; mdbook linkcheck green.

## Forward look

Still in the arc's item 3: near-zero-code **conventions** — a decision-record shape
(Context/Decision/Consequences/Alternatives/Status) over `thread_results` + the
`supersedes` edge, and an `ack` grounding act as a reserved vote value (version-pinned).
Then as-of context replay → seed-from-message → context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
