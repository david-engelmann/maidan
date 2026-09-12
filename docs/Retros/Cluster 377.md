# Cluster 377 retro — durable projector egress (row #38, the result-delivery foundation)

The Slack (309) and GitHub (312) projectors posted inline and best-effort. A
transient 502 lost the message with a `tracing::warn!` and nothing else: no
retry, no queue, no operator surface, no event. That was tolerable while the
projectors carried ordinary chat, and it stops being tolerable the moment an
*agent's result* rides the same path — "recorded and auditable per target" is not
implementable on log-and-drop.

So this cluster is the foundation the rest of the result-delivery arc stands on
(Clusters 378–381, pinned in [Result Delivery](../Result%20Delivery.md)). It is
Open Work row **#38** — `NEW-webhook-health`, *"webhook retry-then-disable on
projector egress + a loud `ProjectorMisconfigured`/DLQ, never silent drop"* —
promoted out of Wave 4 because the arc cannot start without it. A reorder, not
new scope.

**A queue, not a new connector.** Nothing about what the projectors say, where
they say it, or who may link a channel changed. Only *how a post survives
failure* changed.

## What shipped

- **377.1 (#777) — the store foundation.** `maidan_egress_outbox` (pg 0082 /
  sqlite 0081), modelled on `maidan_mail_outbox` (0050): workspace + thread FKs,
  a `source_log_id` with **no FK** (so event-log retention pruning cannot cascade
  into a queued delivery), the `(surface, selector)` destination pair, and the
  `pending`/`delivered`/`dead` + `attempts` + `next_attempt_at` scheduling
  columns. `EgressSurface` / `EgressTarget` / `NewEgressOutbox` / `EgressOutbox`
  in `maidan-types` + `EgressStore` on both backends — `enqueue` (`ON CONFLICT DO
  NOTHING`) / `claim_next_due` (Postgres `FOR UPDATE SKIP LOCKED`, SQLite
  serialized CAS) / `mark_delivered` / `mark_failed` (reschedule or dead-letter)
  / `count_dead`. Zero-blast-radius: no worker, no routes, no events.
- **377.2 (#778) — the worker.** `egress_worker.rs`, the sibling of
  `mail_worker.rs`: lease 120 s, backoff 30 s → 1 h, dead-letter at 8 attempts,
  1000 posts per tick, tick interval `MAIDAN_EGRESS_WORKER_TICK_SECS` (default
  5 s). `route_message_to_{slack,github}` now **enqueue** instead of posting; the
  link lookup and the loop-prevention metadata check stay at enqueue and only the
  post moved. Spawned in `main.rs` when a projector sender is configured — which
  is exactly when the projectors enqueue, so an unconfigured deployment neither
  queues nor drains and configuring a sender turns both halves on together.
- **377.3 (#779) — retry-then-disable + `ProjectorMisconfigured`.** A revoked
  Slack token or a deleted GitHub issue is not a transient fault, but 377.2's
  worker treated it as one: eight doomed attempts per message, forever, with the
  DLQ filling one row per message and nobody told. An auth/config-class failure
  now **disables the link** and says so. `disabled_at` on both link tables (pg
  0083 / sqlite 0082, `NULL` = enabled) + `disable_{slack_channel,github_issue}_link`
  on both backends, idempotent so only the first failure announces it; the
  enqueue skips a disabled link, so the queue stops growing per message.
  `Event::ProjectorMisconfigured { workspace, channel?, thread, surface,
  selector, error }` (the full EventKind drill, non-federatable) +
  `maidan_egress_deliveries_total{surface,outcome}` (`sent`/`retry`/`dead`/
  `disabled`/`unroutable`). Re-linking clears `disabled_at` — the re-enable path
  is the gesture an operator already knows.
- **377.4 (#780) — the operator DLQ.** `DeadEgress` + `list_dead_egress` /
  `requeue_dead_egress` on both backends, over `GET /operator/egress/dead` and
  `POST /operator/egress/dead/{id}/requeue` in a new `routes/egress_ops.rs`, both
  **`token:admin`** (the Cluster-306 mail-DLQ shape — the egress queue is
  cross-workspace, so a workspace-scoped token has no business reading it). A row
  carries `surface` + `selector` + `thread_id` + `attempts` + `last_error`, which
  is the whole point: *what failed, where was it going, and what did the surface
  actually say* without a log dive.
- **377.5 — this retro + the doc-close.**

## Decisions

- **The mail outbox is the model, not a new abstraction.** `maidan_mail_outbox`
  (304–306) already answers claim-safely-across-replicas, back off, dead-letter,
  let an operator replay. Egress needed the same four answers, so it got the same
  four shapes — one table, one worker, one DLQ pair — rather than a generalized
  "delivery" layer over both. Two concrete queues that read alike beat one
  abstract queue that neither surface fits.
- **`(surface, selector)` is stored as text, and the claim hands the worker the
  raw pair.** A row whose selector does not decode has to *reach* the worker to
  be dead-lettered; a claim that filtered on decodability would lease it forward
  forever and wedge the queue behind it. Decoding is therefore the worker's job,
  and an undecodable destination dead-letters on sight rather than burning eight
  attempts against a row no sender can ever address. The pair is also exactly
  what Cluster 378's allowlist will key on, which is why it is a pair and not an
  opaque blob.
- **`UNIQUE (source_log_id, surface, selector)` is load-bearing, not hygiene.**
  Every replica runs the notification router's bus consumer, so all of them
  enqueue — the Cluster-238 lesson. Without the index a 3-replica deploy posts
  every comment three times.
- **`source_log_id` carries no FK.** Event-log retention pruning (Cluster 186) is
  allowed to delete the row that caused a send. A cascade there would silently
  delete a queued delivery, which is the exact failure this cluster exists to
  remove.
- **Disable the *link*, not the projector.** A bad token breaks every link the
  projector holds, so the projector-wide reading is arguably the truer one. It is
  also a much bigger blast radius to arm automatically, and each link will
  disable itself as its own delivery fails — correct per-link, at the cost of an
  operator re-linking each one. A projector-wide kill switch is a decision for a
  human, not an inference from one 401.
- **A rate-limited 403 is explicitly not a misconfiguration.** This is the
  sharpest edge in the cluster. GitHub answers a *secondary rate limit* with
  **403** — the same status as a revoked token — so classifying on status alone
  would disable a healthy link during a traffic spike, and only an operator
  noticing and re-linking would undo it. `GithubError::Api` became a struct
  variant carrying `rate_limited`, read from `x-ratelimit-remaining: 0` /
  `retry-after`.
- **The misconfiguration predicate is a per-surface allowlist, not a status
  range.** GitHub answers by status (401/403/404); Slack answers *logically*, in
  an error string (`invalid_auth`, `token_revoked`, `missing_scope`,
  `channel_not_found`, `not_in_channel`, `is_archived`, …) with HTTP 200 over the
  top. Both surfaces overload their most severe-looking signal for their most
  transient failure. An error code Slack adds later is therefore **retried**, not
  treated as fatal — the unknown case defaults to the recoverable one.
- **Ingress is untouched by a disable.** A revoked *write* scope does not stop
  Slack or GitHub from reaching us, and silencing inbound conversation over an
  outbound credential problem would lose real messages to fix a delivery bug.
- **Re-linking is the re-enable path; there is no new route.** `POST
  /workspaces/:wid/{slack,github}-links` already upserts, and the upsert now
  resets `disabled_at`. A second way to do it would be a second thing to get
  wrong.
- **`ProjectorMisconfigured` is non-federatable.** A peer has no standing to
  declare *our* connector credentials broken. The `error` field is the surface's
  own words, because "check your Slack config" is not a diagnostic and
  `channel_not_found` is.
- **The DLQ is `token:admin`, and the per-surface counters kept their meaning.**
  `maidan_{slack,github}_egress_total` stay a count of *post attempts*, now
  recorded at the post site inside the worker; the new
  `maidan_egress_deliveries_total{surface,outcome}` is the queue-level companion.
  Redefining an existing counter to mean "queued" would have broken every
  dashboard built on it.
- **Opt-in at the sender, not at a feature flag.** The worker spawns only when a
  projector sender is configured, and `sweep_once` is a no-op without one — so an
  unconfigured `AppState` cannot burn a queued row's attempts, and CI's
  unconfigured deploys stay byte-identical.

## Surprises

- **GitHub's rate limit is a 403.** The first cut of 377.3 classified on status
  alone and would have disabled a link during a traffic spike — exactly the
  outage-amplifying behaviour the cluster exists to prevent, since the "fix" is a
  human noticing. It forced `GithubError::Api` from a tuple variant to a struct
  variant, which is a better shape anyway.
- **Slack reports a fatal credential error with HTTP 200.** The two connectors do
  not merely differ in their error vocabulary; they differ in *which layer* the
  error lives in. There is no cross-surface predicate to write, only two
  unit-tested ones.
- **The DLQ it was modelled on has no e2e.** The mail DLQ routes (306) are
  covered by a store test and the capability matrix, and never exercised over
  HTTP. Matching that precedent exactly would have shipped 377.4's routes without
  ever calling them, so it adds the e2e the older surface never got.
- **Building the DLQ fixture as enqueue → claim → fail is what makes the
  assertion mean anything.** A hand-written `dead` row would have asserted an
  `attempts` number the test itself chose. Going through a real claim proves the
  DLQ reports the attempt count the *worker* actually burned.
- **Moving the post out of the projector barely moved any code.** The link lookup
  and loop-prevention check stayed where they were; the diff is a `post(...)`
  becoming an `enqueue(...)`. The hard part was never the send — it was that
  there was nowhere for a failure to live.

## Test evidence

- Store, both backends: `egress_outbox` (enqueue + dedup, claim/lease, deliver,
  reschedule vs dead-letter, `count_dead`, and the 377.4 DLQ arc — the dead entry
  listed with its destination and error, requeue emptying the DLQ and making the
  row claimable with `attempts` reset, a second requeue returning `false`);
  `slack_links` / `github_links` extended (disable flips the flag, a second
  disable returns `false` and preserves the original timestamp, re-linking
  clears it). `dialect_parity` + `backend_parity` + `concurrent_migrations` green.
- Pure units: the `EgressTarget` round-trip and rejection grammar (a repo name
  containing a `#` still splits on the last one; issue numbers start at 1; an
  unknown surface does not decode), and both `is_misconfiguration` predicates —
  credential and destination errors disable; rate limits, outages, transport
  errors and unknown future codes retry; `403` flips on the rate-limit flag
  alone.
- Server: `egress_worker_e2e` (deliver-once, reschedule-not-drop, an unroutable
  destination dead-lettering without a post, an unconfigured sender leaving the
  queue alone, and `a_misconfigured_link_is_disabled_announced_and_stops_queueing`
  — one post attempt, `disabled_at` set, the `ProjectorMisconfigured` on the bus
  *and* durable in `maidan_events`, a later message queueing nothing at all, and
  re-linking restoring delivery); `egress_wire_e2e` driving the **real**
  `GithubApiClient` against a loopback 403 carrying `x-ratelimit-remaining: 0` to
  prove the header plumbing end-to-end; `egress_dlq_e2e` **auth-enabled** (the
  operator's actual loop, plus `403` for a `workspace:read` token on both
  routes — `token:admin` is the entire surface, so a bypass run would prove
  nothing); the four projector ingress/egress e2es extended with a sweep and a
  second-replica dedup case.
- Contracts: `event-kinds.json` + its golden test cover `projector_misconfigured`;
  `http_capability_map_contract`, `http_openapi_capability_map_contract`,
  `http_capability_matrix_e2e`, `openapi_e2e` bijection.

## Forward look

**Cluster 377 is complete, and Open Work row #38 is closed.** Projector egress is
durable (a queue with retry and backoff), bounded (dead-letter at 8), loud (a
disabled link announces itself as an event and a metric), and operable (a
`token:admin` DLQ with replay). Nothing is silently dropped.

Deferred (follow-ups): a `/ui` panel for the DLQ (the Cluster-137 operator tab is
the natural home, but that is UI work); bulk requeue; a DLQ-depth alert rule
(`maidan_egress_deliveries_total{outcome="dead"}` is the signal, and wiring an
alert belongs with the SLO rules); an automatic re-enable probe; and a
projector-wide disable.

**Next: Cluster 378 — the trust boundary + the sender upgrade.** `maidan_egress_targets`
(a per-workspace allowlist of `{surface, selector}`, default empty ⇒ deliver
nowhere) so an agent-supplied `deliver_to` *selects* while an operator
*authorizes*; the sender trait returning an `ExternalRef` plus `update_message`,
without which an idempotent update-in-place is unimplementable; and a pure
`egress_body` projection (mention neutralization, truncation, GFM → Slack
mrkdwn). See the "Result delivery — the external last mile" section of
[[Open Work]] and [Result Delivery](../Result%20Delivery.md).

## Acknowledgements

Four impl PRs (#777 the outbox store → #778 the worker → #779 retry-then-disable
+ `ProjectorMisconfigured` → #780 the operator DLQ) + this retro, on the
foundation-then-wire + new-route-preflight + full-EventKind-drill patterns.
