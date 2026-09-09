-- Cluster 358 (T1/T5, mirror of postgres 0063): a per-thread budget envelope.
-- Optional maxima + accumulated usage; USD as integer micros; wall time derives
-- from the Cluster-351 working clock. A side table keeps `row_to_thread` untouched.
CREATE TABLE IF NOT EXISTS maidan_thread_budgets (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    max_tokens INTEGER,
    max_usd_micros INTEGER,
    max_turns INTEGER,
    max_wall_secs INTEGER,
    used_tokens INTEGER NOT NULL DEFAULT 0,
    used_usd_micros INTEGER NOT NULL DEFAULT 0,
    used_turns INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
