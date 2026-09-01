# Cluster 349.0 retro — deferred-work wrap-up (audit close-out)

> Tag **`v349.0.0`**. Phase XXIV (post-gate hardening). **Cluster 18 of the post-flagship
> audit program — and its close-out.** No new gate tag.

A **multi-PR cluster** (349.1–349.5 + this retro), unlike the 1:1 cadence of 332–348 — the
maintainer asked to "wrap it up" by closing *every* remaining deferred item from the post-flagship
audit in one cluster. Each sub-PR shipped and CI-verified on its own; this retro cuts the single
`v349.0.0` tag.

## What shipped

- **349.1 — Store trait split.** The 258-method `maidan_store::Store` god-trait split into **35
  cohesive domain sub-traits** (`WorkspaceStore`/`ThreadStore`/`MessageStore`/`NotificationStore`/…)
  along the existing doc-commented sections (no method reordering). `Store` is now a method-less
  super-trait with a blanket impl, so every `dyn Store` call site is unchanged; a caller can now bound
  on a narrow sub-trait. A new `maidan_store::prelude` re-exports `Store` + every sub-trait for
  concrete-backend callers. 181 test/bench imports migrated to the prelude glob. Pure interface reorg.
- **349.2 — notification multi-row batch INSERT.** `Store::create_notifications_batch` writes the
  whole unmuted follower set in ONE `INSERT … ON CONFLICT DO NOTHING RETURNING` (Postgres `UNNEST`,
  SQLite chunked `VALUES`). With 348 the `MessagePosted` fan-out is now ~2 store round trips
  regardless of follower count.
- **349.3 — LSN read-replica routing CI job.** A `replica-routing` job stands up a real
  primary+streaming-standby pair (`scripts/replica-harness.sh`) and runs the three previously
  `#[ignore]`d LSN routing tests — the read-your-writes contract is now proven in CI, not just
  locally. (Self-validated: the job passed on its own PR.)
- **349.4 — MCP projector link-management tools.** The six MCP twins of the Cluster-346 REST link
  routes (Slack + GitHub link/list/unlink). Capability-filtered, so no catalog bloat (91 tools).
- **349.5 — SMTP wire-path test.** The real `lettre` `SmtpTransport` proven to speak SMTP end-to-end
  against an in-process plaintext sink (no Docker, no MTA) — closing the "transport never exercised
  on the wire" gap.

## Deferrals — closed as decisions, not left dangling

The audit's remaining optional tails were assessed; two are **declined with rationale** so nothing is
left as an open "revisit":

- **Broad MCP arg-defaulting (`author_id`/`member_id` ← `auth.member_id`) — DECLINED.** The audit
  itself deferred this as low-priority; on inspection it's a semantic change to the critical hero-loop
  post path (the `post_message`/`post_dm_message` handlers don't take `auth`; it would make a required
  arg optional and change tool schemas) for modest ergonomic gain that `whoami`/`GET /me`
  self-discovery already covers. Not worth the blast radius on the most-used write. Revisitable on
  demand.
- **Cross-crate context-assembler hoist into `maidan-router` (P1.2 tail) — DECLINED.** The audit noted
  the remaining payoff is only ending the `as_of` double-impl, and the trickiest shared logic already
  routes through `maidan_types::reconstruct_messages_through`. The hoist is blocked by a `ThreadContext`
  name collision + utoipa-feature propagation + a `futures` dep — multi-crate surgery for near-zero
  user value. Declined until the two assemblers actually drift.
- **README visual media — needs a recorded asset (maintainer).** A terminal GIF/screencast can't be
  authored in-repo; left for the maintainer to record. Not a code gap.

## Surprises / decisions

- **The trait split is invisible to `dyn Store` callers but not to concrete-backend callers** — those
  need the *declaring* sub-trait in scope, which the `prelude` absorbs. The blanket impl must stay
  `Sized` (no `?Sized`) so it doesn't collide with the compiler's auto-impl for `dyn Store`.
- **A subtlety in the split's safety proof:** per-boundary method *sets* were verified identical across
  the trait and both impl blocks *before* the transform, so the compiler-checked placement couldn't
  silently misfile a method.
- **The batch INSERT's `DO NOTHING` only ever dedups against existing rows** — the fan-out recipient
  set is distinct members at one `source_log_id`, so there's no intra-batch conflict key.
- **The LSN CI job uses host Docker, not `docker:dind`** — the harness maps container ports to
  `localhost`, which the cargo-test process must reach directly.

## Test evidence

Full `cargo test -p maidan-store` (both backends incl. the new `run_batch_suite` against a real
Postgres container); `notification_router_e2e`; the three LSN routing tests green in CI against real
streaming replication; `projector_link_tools_slack_and_github`; `smtp_transport_delivers_a_message_over_the_wire`;
both MCP contract-sync tests (91 tools); fmt + strict clippy + `--all-targets` + bootstrap-strip +
agent-contract golden clean across all five PRs.

## Forward look

**The post-flagship audit program (Clusters 332–349) is complete** — every P0/P1/P2 item is shipped or
closed as a documented decision. No audit item remains open. Remaining backlog is the standing,
explicitly-gated tracks (public launch, hosted `/play`, provider recipes) and the two declined tails
above, revisitable on demand.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes the post-flagship audit
program ([[Open Work]]).
