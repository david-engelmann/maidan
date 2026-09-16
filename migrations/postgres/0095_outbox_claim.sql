-- Cluster 398.1: a claim lease on the outbox relay.
--
-- `list_pending` took no lock, and the relay is spawned in EVERY replica
-- (`validate_startup` refuses to disable it in production). So on any HA
-- deployment every replica selected the same relayable rows, published each to
-- the bus, and only then marked them published — relaying every event N times.
--
-- Downstream that is not a benign duplicate: `maidan_webhook_deliveries` has no
-- unique on (subscription_id, log_id), so each tenant endpoint got N POSTs, and
-- `fsm_hook_worker` re-fired every hook through `dispatch_mcp_tool` with
-- `AuthContext::bypass()`.
--
-- The lease is the same shape the codebase already uses for thread claims
-- (Cluster 192) and task schedules (Cluster 227): claim atomically with
-- FOR UPDATE SKIP LOCKED, work outside the transaction, and let an expired
-- claim be reclaimed so a crashed relay cannot strand a row. At-least-once is
-- preserved — a claim is not a publish.
ALTER TABLE maidan_outbox
    ADD COLUMN claimed_at TIMESTAMPTZ;

-- The claim scan reads unpublished rows ordered by id; keeping claimed_at in
-- the partial index lets the "unclaimed or expired" predicate stay index-only.
CREATE INDEX IF NOT EXISTS idx_outbox_claimable ON maidan_outbox (id, claimed_at)
    WHERE published_at IS NULL AND quarantined_at IS NULL;
