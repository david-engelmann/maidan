# Cluster 354 retro — the wait contract (hardening `wait_for_*`)

Wave 1 #5 (H4). The `wait_for_*` long-polls (mention, notification, result,
ready, claim_expired) were **live-only**: each subscribed to the event bus and
parked, so a signal that fired between the caller's last drain and the subscribe
was silently missed until the next one — the classic drain/subscribe race. An
agent that does "drain my inbox, then wait for the next thing" could lose the
very event that arrived in the gap. This cluster closes that race and pins down
the surrounding contract (what happens on resume, and how a wait interacts with
occupancy).

## What shipped

- **354.1 (#638) — lookback on the member waits.** `wait_for_mention` /
  `wait_for_notification` gained an optional `since_log_id` (the caller's
  high-water `log_id` from its last drain). After subscribing, the wait replays
  the durable log for a matching event with `log_id > since_log_id` before
  parking live. `lookback_member_event` reuses `Store::list_events_after`,
  matching `kind` + `member_id()` from the payload, RBAC-filtered like the live
  path.
- **354.2 (#639) — lookback on the thread/workspace waits.** The same for
  `wait_for_result` / `wait_for_ready` / `wait_for_claim_expired`, via a shared
  `lookback_event` that filters on the `StoredEvent` columns (kind, channel_id,
  thread_id) before deserializing, optionally RBAC-filtering. `wait_for_result`
  returns the current `get_thread_result` payload (its live return shape); the
  other two return the event.
- **354.3 (#640) — the wait contract.** Codified the two remaining H4 concerns,
  both already-holding invariants: a comment at `PresenceRegistration::drop`
  (the tree's only `Drop`) fixing the **no-occupancy-I/O-in-Drop** invariant in
  place; and an Integration.md "Long-poll waits" section documenting the
  agent-author contract — resume with `since_log_id`, make pre-wait side effects
  idempotent, and **evict-on-wait** (a wait doesn't renew the claim lease, so a
  waiter that outlives its lease is reclaimed).

## Decisions

- **Subscribe before the lookback.** The ordering is the correctness argument:
  subscribing first means an event committed after the lookback query is caught
  live, and one committed before it is caught by the replay — no gap. An event
  caught by both returns once (the lookback wins; the live stream is dropped).
- **Reuse `list_events_after`; no new store surface.** The lookback is a
  workspace-scoped log scan filtered in Rust on the `StoredEvent` columns +
  event accessors. Paged at 256. Nothing new in the `Store` trait.
- **`since_log_id` is optional.** Omitted, every wait behaves exactly as before
  (pure live). The lookback is opt-in for the caller that tracks a high-water.
- **evict-on-wait needs no new mechanism.** The claim lease (Cluster 351)
  already makes a non-renewing holder reclaimable; a wait simply doesn't renew,
  so a stuck waiter is evicted by the existing reclaim. The cluster documents
  this rather than adding a second path.

## Surprises

- **The toolchain file got a stray append mid-work**, which made a local
  `cargo fmt` run under the wrong rustfmt and let two unformatted spots into
  354.1's first push — CI's `fmt --check` caught them (`server.rs`, `member.rs`).
  Reverted the file, re-formatted under pinned 1.91, amended, force-pushed green.
- **The coverage job hit the known `context_query_count_e2e` flake** (small=9 /
  large=8, connection warm-up) on 354.1 — unrelated to the change (all new wait
  tests passed); coverage is non-required, so it didn't gate the merge.

## Test evidence

- `wait_for_mention` / `wait_for_ready` / `wait_for_claim_expired` e2es each
  assert: a pre-subscribe signal is caught via `since_log_id`; an already-seen
  high-water returns `null` promptly; and a pre-subscribe signal in a private
  channel is RBAC-filtered to `null`.
- `result_tools_set_get_wait_and_aggregate` asserts a lookback from the log head
  returns dep1's already-set result payload.
- The catalog contract holds (all five waits document `since_log_id`); fmt +
  strict/all-targets clippy green on each sub-PR.

## Forward look

**H4 is complete.** The wait race is closed with an opt-in, RBAC-preserving,
gapless lookback, and the resume/idempotency/evict-on-wait contract is written
down. Next-ranked is **Wave 1 #6 — W1** (a durable human owner ≠ claimer; the
claimer cannot merge its own PR; persist steer; notify the owner on stuck).

## Acknowledgements

Built as a three-PR stack (#638 → #639 → #640) on the 351 occupancy clocks,
each rebased onto `main` as its parent merged.
