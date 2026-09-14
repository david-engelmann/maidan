# Cluster 379 retro — the result-delivery primitive

Clusters 377 and 378 made projector egress durable, aimable, and repeatable.
This cluster is the producer's actual ask: a structured thread result reaches a
blessed GitHub PR comment or Slack message, durably, once, and a re-review
edits that object instead of stacking a second one.

Five impl PRs (379.1–379.5) + this retro. **The grammar is frozen** at
`maidan.waiter.result/1`. Additive fields are free. A change to the meaning of an
existing field, or to the `deliver_to` shape, needs a new `schema` value —
Maidan routes on the discriminator, and an unrecognized one is inert rather
than mis-delivered. That confirmation is the note back to the producer.

**Cluster 380 is unparked as next.** The 379.2 fixture lock already carries
`head_sha` (additive on the frozen `maidan.waiter.result/1` schema;
`parse_waiter_result` ignores it today — 380 reads it). Remaining care, not a
park: the `line_range` frame of reference (file-absolute post-image lines vs
diff-relative) is still unstated. 380.1 must pin that frame **and** pass the
envelope's `head_sha` as `commit_id`, never the live PR head.

## What shipped

- **379.1 (#787) — result-delivery state.** `maidan_result_deliveries` (pg 0085 /
  sqlite 0084), one row per `(thread_id, surface, selector)`. This table is
  *intent and identity*; Cluster 377's outbox is *transport*. Three questions
  the queue cannot answer: who delivers (every replica sees `ThreadResultSet`,
  exactly one replica must enqueue), is this new (a re-review vs a replayed
  event), and update what (`external_ref` = a Slack `ts` or GitHub comment id).
- **379.2 (#788) — the contract lock.** `maidan_types::waiter::parse_waiter_result`,
  a pure tolerant reader of `{schema, result_kind, status, deliver_to[],
  rendered, summary, view_url, pr}` with a `DeliverTarget::Unknown(String)`
  arm. Unit-tested against
  `crates/maidan-types/tests/fixtures/waiter_result_v1.json` — a
  producer-side grammar change breaks a test in this crate rather than a
  delivery in production. Unrecognized `schema` ⇒ no delivery attempted.
- **379.3 (#789) — the trigger.** A `ThreadResultSet` arm in
  `notification_router::route_event` delegates to `result_delivery.rs`: fetch
  → parse → per target, allowlist-check then enqueue. Empty `deliver_to` ⇒
  zero rows (valid). Non-`reviewed` ⇒ a short **Maidan-authored** failure
  notice from `status` alone — never silence, never a clean pass. Skip is a
  recorded normal outcome (`status=skipped`), not an error. Metric
  `maidan_result_deliveries_total{outcome=enqueued|skipped}`.
- **379.4 (#791) — update-in-place.** Stored `external_ref` → `update_*`;
  absent → post. GitHub recovery marker `<!-- maidan:result:<thread_id> -->`
  at byte 0 of the body (reserved inside the 65536-character ceiling). Slack
  has no HTML comment; a lost Slack ref posts again. `EgressKind`
  `{Projector, Result}` on the outbox (pg 0086 / sqlite 0085) so a projector
  `MessagePosted` aimed at the same GitHub issue cannot PATCH the result
  comment. Result 401/403/404 **dead-letters without `disable_link`** — a
  result must not take down a projector issue-link. Transient outbox failure
  does not `mark_result_delivery_failed`; dead-letter does. Success writes
  `mark_result_delivered(id, handle, armed_revision)`.
- **379.5 (#793) — status + replay.** `GET /threads/:id/deliveries`
  (`workspace:read` + thread access) + `POST …/deliveries/:did/replay`
  (`workspace:write`, bodyless) + MCP `list_result_deliveries` /
  `replay_result_delivery`. Audit per attempt (`result_delivery.attempt`,
  `actor_id: None`) and per replay (`result_delivery.replay`). Replay
  **cannot** call `arm_result_delivery` (that only wins on a newer revision)
  and cannot reuse the original `source_log_id` (the outbox unique key would
  no-op); it reopens the row as `pending` without bumping `armed_revision`
  and enqueues with a synthetic negative log id. An unblessed replay stays
  skipped — delivery status is not allowlist policy.
- **379.6 — this retro + the doc-close.**

## Decisions

- **Two revision watermarks — the approved deviation from the investigation.**
  The Open Work sketch armed with `ON CONFLICT DO UPDATE … WHERE produced_at >
  delivered_revision`. That predicate cannot tell a second replica of the
  *same* revision (both read `NULL`, one must lose) from a genuinely *newer*
  result arriving while a delivery is still in flight (must win, or that
  result is silently dropped). Arming is therefore a **single monotonic test
  against `armed_revision`** (`revision > armed_revision` wins; the winner
  keeps `external_ref`). `delivered_revision` stays a truthful record of what
  actually reached the surface, and may lag `armed_revision` while a send is
  in flight. Comparing only against `delivered_revision` would collapse those
  two cases. Documented on the type
  (`crates/maidan-types/src/result_delivery.rs`) so the next reader does not
  "simplify" it back to one column.
- **`maidan_result_deliveries` is not the outbox.** Dedup/identity live here;
  retry/backoff/dead-letter stay on `maidan_egress_outbox`. Folding them would
  either make projector traffic carry result-only columns or make result
  identity ride a transport key that retention can drop. The outbox unique
  key stays `(source_log_id, surface, selector)`; the delivery unique key is
  `(thread_id, surface, selector)`.
- **Empty `deliver_to` is success.** Zero rows, HTTP 200 `[]`, no skip rows.
  "Delivered nowhere" is the pinned contract's supported outcome, not a
  missing-handler bug. An unknown surface or an unblessed target *does* write
  a `skipped` row, so the producer can read *why*.
- **Non-`reviewed` still delivers, as a Maidan-authored notice.** Silence
  would look like a hang. Shipping `rendered` would look like a clean pass.
  The notice is built from `status` alone. On GitHub it still carries the
  recovery marker, so a later `reviewed` result updates that comment rather
  than stacking a second one.
- **GitHub gets `rendered`; Slack gets `summary`.** Unchanged from 378.3, now
  actually called. The marker is GitHub-only (HTML comments are a GitHub
  rendering accident; Slack would show them). Marker bytes are reserved
  *inside* `GITHUB_BODY_MAX_CHARS` so a max-size `rendered` plus the marker
  cannot 422.
- **`EgressKind` is load-bearing, not decorative.** Update-in-place keys on
  the result-delivery row's `external_ref`. Without a kind, a projector
  `MessagePosted` to the same issue would PATCH the result comment. Existing
  outbox rows default `projector`. Projector senders still always `post_*`.
- **Result misconfiguration does not disable a projector link.** A 401/403/404
  on a result comment is this delivery's problem. `disable_link` would stop
  Cluster-309/312 room-to-issue relay for every subsequent message. Result
  auth-class failures dead-letter the outbox row and `mark_result_delivery_failed`.
- **GitHub marker recovery lists comments only when `delivered_revision` is
  set.** A first post that lost its handle has nothing to find; listing would
  be a guess. Slack never recovers — a lost `ts` posts again (the 378.2
  `Ok(None)` cost).
- **Replay is not re-arm.** `arm_result_delivery` is the "is this new?"
  predicate. Operator replay of the *same* revision would always lose it.
  `prepare_result_delivery_replay` sets `status=pending`, clears `last_error`,
  and leaves `armed_revision` + `external_ref` alone so the worker still
  updates in place. The new outbox row uses a synthetic negative
  `source_log_id` (`-timestamp_nanos`, retry +1 up to 8) so it cannot collide
  with event-log ids. The worker rebuilds the body from the current thread
  result (`parse_waiter_result` + `delivery_body`); the snapshot is a
  fallback if the envelope is gone.
- **Unblessed replay stays skipped (200, not 403).** Allowlist policy is
  `token:admin`. A `workspace:write` caller may ask; they may not bless.
  Returning the skipped row is how the producer learns the target is still
  unblessed. Unroutable (unknown surface) is 400 / MCP `InvalidParams`.
- **Audit never fails the delivery.** Worker `result_delivery.attempt`
  (`actor_id: None`, metadata includes `outcome` + `egress_id`) and replay
  `result_delivery.replay` (REST `actor_id: Some(auth.member_id)`) are
  best-effort. A failed audit write logs `audit.write_failed` — the Cluster-182
  mint-must-not-lose-its-secret polarity, applied to a send that already
  happened.
- **Result-delivery reads stay on the primary.** Same carve-out as the
  allowlist: a lagging replica must not hide a row an operator just replayed,
  and `is_egress_target_allowed` on the replay path is an authorization check.

## Surprises

- **One watermark is not enough, and it only became obvious in the store
  test.** The investigation's `produced_at > delivered_revision` looks like
  the natural idempotency clause until two replicas arm the same first
  revision (both see `delivered_revision IS NULL`) *and* a newer result
  arrives while the winner's send is in flight (must not lose to
  `delivered_revision` still being NULL). Those are opposite CAS outcomes
  on the same column. Splitting `armed_revision` / `delivered_revision`
  is the smallest type that can say both.
- **The outbox unique key made replay a new problem, not a reuse of arm.**
  Re-enqueue with the original `source_log_id` is a silent no-op if the dead
  row still exists. Deleting that row to make room would lose the DLQ
  record. Synthetic negative ids are ugly and correct; event-log ids are
  small, positive, and monotonic.
- **`clippy::await_holding_lock` does not honour `drop(guard)`.** The lock
  has to end its *lexical* scope before `.await`. A block `{ let g = …; use
  g; }` then await, not an explicit drop. Cost a lint round on the worker.
- **A stacked PR targeting a deleted base is closed, not retargeted.** 379.4
  originally stacked on 379.3's branch (#790). Squash-merge deleted that
  branch (`base_ref_deleted`) and GitHub closed #790. The replacement is
  #791, rebased onto `main`. Stacked result-delivery PRs should retarget
  onto `main` the moment the parent squash-lands, before the branch is
  deleted.
- **CI workflows only run on `pull_request` to `main`.** A PR whose base is
  another feature branch reports no checks. Stack while the parent runs;
  retarget to get the eight required jobs.

## Test evidence

- Store, both backends: `result_deliveries` — first arm wins, same-revision
  second replica loses, a newer revision re-arms and keeps `external_ref`,
  skip fingerprint writes `skipped` without enqueue, `mark_result_delivered`
  sets `delivered_revision` to the armed watermark, `mark_result_delivery_failed`
  does not. `run_replay_suite` — prepare does not bump `armed_revision` or
  drop the handle; unblessed replay stays skipped and does not enqueue;
  blessed replay enqueues `EgressKind::Result`; unknown id is `None`.
  `dialect_parity` + `backend_parity` + `concurrent_migrations` green.
  Postgres testcontainers skip when Docker is unavailable (this VM);
  sqlite is the complete suite here, Postgres in CI.
- Types: `parse_waiter_result` against the committed fixture (the contract
  lock) + unknown-schema inert + `DeliverTarget::Unknown` skip +
  `to_egress_target` refusals (name-not-id, `pr <= 0`, missing slash).
  `ResultDelivery::target` / `reference` decode; a malformed handle rebuilds
  nothing.
- Server: `result_delivery_e2e` — empty `deliver_to` writes nothing; blessed
  GitHub enqueues; unblessed writes `skipped`; unknown surface writes
  `skipped`; non-`reviewed` enqueues a failure notice and never the
  producer's `rendered`. `result_delivery_update_e2e` — first send posts and
  stores the handle; second revision PATCHes; GitHub 404 on update recovers
  via the marker when `delivered_revision` is set; projector rows still
  always post. `result_delivery_status_e2e` **auth-enabled** (minted token
  with `workspace:read`+`write`+`thread:transition`) — list empty → route
  blessed GitHub → pending → `sweep_once` delivered + attempt audit → POST
  replay pending (`armed_revision` unchanged, handle kept) → replay audit as
  the minted member → sweep updates in place; unblessed replay stays skipped;
  discord unroutable → 400; unknown id → 404.
- MCP: `result_delivery_tools_list_and_replay` (list empty / replay
  unblessed stays skipped / blessed enqueues `EgressKind::Result`); catalog
  + capability-map contracts sorted (`list_result_deliveries` before
  `list_reviews`; `replay_result_delivery` after `renew_claim`).
- Worker: result 401/403/404 dead-letters without disabling the issue-link;
  projector misconfiguration still disables. Marker reserved in the GitHub
  budget. `clippy::await_holding_lock` clean.
- Contracts: `openapi_e2e` bijection (`ResultDelivery` schema + two path
  stubs), `http_capability_map_contract`, `http_capability_matrix_e2e`
  (`{did}` substitution in the `/threads/` branch; POST bodyless so no
  matrix body clause). `mdbook build` with the linkcheck renderer, since
  `docs/Result Delivery.md` is published.

## Forward look

**Cluster 379 is complete.** A producer writes `maidan.waiter.result/1` onto a
thread; Maidan delivers `rendered` to a blessed GitHub issue and `summary`
to a blessed Slack channel; a re-review updates the same object; the
producer reads per-target disposition over REST + MCP and an operator can
replay.

Deferred (follow-ups): a `/ui` deliveries panel; MCP twins of the allowlist
routes (still declined — an agent has no business editing the boundary that
constrains it); wildcard / org-level selectors; Slack Block Kit; recovering
a lost Slack `ts` without re-posting.

**Next: Cluster 380 — inline per-finding PR review comments** (unparked: the
fixture carries `head_sha`; 380.1 still has to pin the `line_range` frame of
reference and must not resolve the PR head at delivery time). **Cluster 381 —
`result_kind` facet + the pinned spec** (half of Open Work row #24): facet
on the **namespaced string**, keep [Result Delivery](../Result%20Delivery.md)
in step, register the envelope in the Wave 3 #30 schema pack. See the
"Result delivery — the external last mile" section of [[Open Work]].

## Acknowledgements

Five impl PRs (#787 the store → #788 the contract lock → #789 the trigger →
#791 update-in-place → #793 status + replay) + this retro. **379.1 and 379.2
were already on `main`** (#787/#788) before 379.3 opened. 379.4's first PR
(#790) was closed by GitHub when 379.3's branch was deleted under it;
#791 is the replacement, rebased onto `main`.
