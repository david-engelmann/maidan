-- Maidan FSM schema, v0004, SQLite dialect.
--
-- SQLite cannot widen an inline CHECK in place; recreate `maidan_threads`
-- with `in_review` allowed, then add the transition log.

PRAGMA foreign_keys = OFF;

CREATE TABLE maidan_threads_new (
    id            TEXT PRIMARY KEY,
    channel_id    TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    title         TEXT,
    state         TEXT NOT NULL DEFAULT 'open'
        CHECK (state IN ('open', 'in_review', 'closed', 'archived')),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    tombstoned_at TEXT
);

INSERT INTO maidan_threads_new
SELECT id, channel_id, title, state, created_at, updated_at, tombstoned_at
FROM maidan_threads;

DROP TABLE maidan_threads;

ALTER TABLE maidan_threads_new RENAME TO maidan_threads;

CREATE INDEX idx_threads_channel ON maidan_threads (channel_id);

CREATE TABLE maidan_thread_transitions (
    id          TEXT PRIMARY KEY,
    thread_id   TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    from_state  TEXT NOT NULL,
    to_state    TEXT NOT NULL,
    actor_id    TEXT NOT NULL REFERENCES maidan_members(id),
    occurred_at TEXT NOT NULL,
    CHECK (from_state IN ('open', 'in_review', 'closed', 'archived')),
    CHECK (to_state IN ('open', 'in_review', 'closed', 'archived'))
);

CREATE INDEX idx_thread_transitions_thread_occurred
    ON maidan_thread_transitions (thread_id, occurred_at);

PRAGMA foreign_keys = ON;
