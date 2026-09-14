-- Cluster 387.1 (Wave 2 #28 run lineage): a thread's home for a producer
-- `run_id`. The value is the producer's string (pi's waiter envelope
-- `run_id`); Maidan does not mint a parallel id. Nested occupancy
-- attributes every open thread that shares `parent_run_id`. F7 mute is
-- a different table and stays orthogonal.
-- Numbered 0090 / sqlite 0089 so this lands after in-flight Cluster 385
-- (Soundcheck, pg 0088 / sqlite 0087) and Cluster 386 (blocked-reason,
-- expected pg 0089 / sqlite 0088) without colliding on the same files.
CREATE TABLE IF NOT EXISTS maidan_thread_lineage (
    thread_id      UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    parent_run_id  TEXT NOT NULL CHECK (parent_run_id <> ''),
    set_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_thread_lineage_parent_run
    ON maidan_thread_lineage (parent_run_id);
