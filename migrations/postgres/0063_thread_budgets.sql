-- Cluster 358 (T1/T5): a per-thread budget envelope. An orchestrator sets any of
-- the optional maxima; an agent reports incremental usage as it works, and when a
-- dimension is exceeded the run is stopped (the claim fails → DLQ, Cluster 358.3).
-- USD is stored as integer micros ($1 = 1_000_000) to keep money out of floats.
-- Wall time is not stored here — it derives from the Cluster-351 working clock
-- (`maidan_threads.work_started_at`) against `max_wall_secs`. A side table (not
-- columns on maidan_threads) keeps the hot `row_to_thread` path untouched.
CREATE TABLE IF NOT EXISTS maidan_thread_budgets (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    max_tokens BIGINT,
    max_usd_micros BIGINT,
    max_turns BIGINT,
    max_wall_secs BIGINT,
    used_tokens BIGINT NOT NULL DEFAULT 0,
    used_usd_micros BIGINT NOT NULL DEFAULT 0,
    used_turns BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
