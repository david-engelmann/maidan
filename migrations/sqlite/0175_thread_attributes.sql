-- Task attributes (Next 33); see the Postgres twin (0175).
--
-- One row per described thread; a thread with no row reads as the defaults
-- (a change, no engine, no threat model), so intake that does not set
-- attributes stores nothing. The row dies with its thread.
CREATE TABLE maidan_thread_attributes (
    thread_id    TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL DEFAULT 'change'
                 CHECK (kind IN ('change', 'plan', 'review')),
    engine       TEXT CHECK (engine IN ('pi', 'session', 'human')),
    threat_model INTEGER NOT NULL DEFAULT 0,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_thread_attributes_engine ON maidan_thread_attributes (engine);
