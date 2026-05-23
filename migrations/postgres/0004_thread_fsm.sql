-- Maidan FSM schema, v0004, Postgres dialect.
--
-- Append-only transition log plus `in_review` in the thread state enum.

ALTER TABLE maidan_threads
    DROP CONSTRAINT IF EXISTS maidan_threads_state_check;

ALTER TABLE maidan_threads
    ADD CONSTRAINT maidan_threads_state_check
    CHECK (state IN ('open', 'in_review', 'closed', 'archived'));

CREATE TABLE maidan_thread_transitions (
    id          UUID PRIMARY KEY,
    thread_id   UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    from_state  TEXT NOT NULL,
    to_state    TEXT NOT NULL,
    actor_id    UUID NOT NULL REFERENCES maidan_members(id),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (from_state IN ('open', 'in_review', 'closed', 'archived')),
    CHECK (to_state IN ('open', 'in_review', 'closed', 'archived'))
);

CREATE INDEX idx_thread_transitions_thread_occurred
    ON maidan_thread_transitions (thread_id, occurred_at);
