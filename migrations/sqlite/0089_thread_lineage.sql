-- Cluster 387.1 (Wave 2 #28 run lineage): SQLite twin of pg 0090.
-- `parent_run_id` is the producer's run identifier as-is (not a minted
-- UUID). Nested occupancy attributes every open thread that shares it.
-- F7 mute (`maidan_thread_mutes`) is a different table and stays orthogonal.
CREATE TABLE IF NOT EXISTS maidan_thread_lineage (
    thread_id      TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    parent_run_id  TEXT NOT NULL CHECK (parent_run_id <> ''),
    set_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_lineage_parent_run
    ON maidan_thread_lineage (parent_run_id);
