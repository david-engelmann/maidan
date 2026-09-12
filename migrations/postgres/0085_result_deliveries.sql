-- Result-delivery state (Cluster 379.1): one row per (thread, target), carrying
-- everything needed to deliver a thread's result to that target **once** and to
-- make a later result an *update in place* rather than a second comment.
--
-- One table does three jobs, which is why it exists separately from
-- `maidan_egress_outbox` (0082). The outbox is *transport* — claim, retry,
-- backoff, dead-letter. This is *intent and identity*:
--
-- 1. **Dedup.** The notification router runs on every replica, so a single
--    `ThreadResultSet` reaches all of them. Without a shared row they would each
--    deliver (the Cluster-238 lesson: a 3-replica deploy triples every
--    delivery). The unique index below plus the conditional upsert mean exactly
--    one replica arms the delivery.
-- 2. **Idempotency across re-runs.** A *newer* result re-arms the row; one we
--    have already seen is a no-op. So re-reviewing a thread edits the existing
--    comment, and a replayed event does nothing.
-- 3. **The external reference.** `external_ref` is the Cluster-378.2
--    `ExternalRef::handle()` — a Slack `ts`, a GitHub comment id. Present means
--    "we have something to edit"; absent means "post, and rely on the hidden
--    body marker to recover".
--
-- **Two revision watermarks, not one.** `armed_revision` is the newest
-- `maidan_thread_results.produced_at` this row has ever *accepted*;
-- `delivered_revision` is the newest it actually *delivered*. Arming compares
-- against `armed_revision`, which is what makes the predicate a single
-- monotonic "is this strictly newer than anything we have seen". Comparing
-- against `delivered_revision` alone cannot distinguish a second replica
-- reporting the same revision (both see `NULL`, so one of them must lose) from a
-- genuinely newer result arriving while a delivery is still in flight (which
-- must win, or that result is silently dropped). One column cannot answer both.
--
-- `surface` + `selector` is the same destination pair `maidan_egress_outbox` and
-- `maidan_egress_targets` (0084) use, at the **delivery** grain
-- (`owner/name#123`), so `EgressTarget::parse` decodes it and
-- `EgressTarget::allowlist_selector()` projects it to the coarser grain the
-- allowlist authorizes on. Deliberately two columns rather than one opaque
-- fingerprint: the row has to stay legible to an operator reading
-- `GET /threads/:id/deliveries`, and the decode path is then shared rather than
-- reinvented.
CREATE TABLE IF NOT EXISTS maidan_result_deliveries (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL,
    -- pending: armed, not yet delivered | delivered | failed | skipped (an
    -- unknown surface, or a target the workspace has not blessed)
    status TEXT NOT NULL DEFAULT 'pending',
    external_ref TEXT,
    armed_revision TIMESTAMPTZ NOT NULL,
    delivered_revision TIMESTAMPTZ,
    attempts BIGINT NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The dedup + idempotency key. One row per destination per thread, forever: the
-- row is updated in place across re-reviews, never appended to, which is what
-- lets `external_ref` survive to become an edit.
CREATE UNIQUE INDEX IF NOT EXISTS idx_result_deliveries_thread_target
    ON maidan_result_deliveries (thread_id, surface, selector);
