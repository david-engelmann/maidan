-- Cluster 401.1: a durable record of who worked a thread.
--
-- Separation of duties on both governance gates (Cluster 375 reviews, Cluster
-- 385 land gate) tests the thread's **live** `assignee_id`. Releasing a claim
-- sets that to NULL, which makes the exclusion vacuous — so an agent could do
-- the work, release, and then approve it as a qualifying third party.
--
-- Assignment history does exist in the event log, but those rows are prunable
-- (Cluster 186 retention), so a gate cannot depend on them. This table is the
-- durable answer to "has this member ever held this thread?": append-only,
-- never cleared by release or unassign, and removed only with the thread it
-- belongs to.
CREATE TABLE IF NOT EXISTS maidan_thread_workers (
    thread_id  UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    first_held_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, member_id)
);

-- The gate asks "did this member work this thread?" one member at a time.
CREATE INDEX IF NOT EXISTS idx_thread_workers_member
    ON maidan_thread_workers (member_id, thread_id);

-- Backfill what is still knowable: whoever holds each thread right now. A
-- release that already happened is gone, so this cannot reconstruct history —
-- it only ensures the gate is no weaker than it was before this table existed.
INSERT INTO maidan_thread_workers (thread_id, member_id)
SELECT id, assignee_id FROM maidan_threads WHERE assignee_id IS NOT NULL
ON CONFLICT DO NOTHING;
