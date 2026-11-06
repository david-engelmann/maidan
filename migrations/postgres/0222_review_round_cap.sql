-- A thread may be sent back from review only so many times before a person
-- decides whether it gets another round. 0 leaves the cap off. A thread with
-- no row still uses the application default of 3.
ALTER TABLE maidan_thread_review_reqs
    ADD COLUMN max_rounds BIGINT NOT NULL DEFAULT 3 CHECK (max_rounds >= 0);

-- `request` is an agent's own ask. `review_round_cap` is the server asking a
-- person to decide after the send-back cap. Existing rows stay requests.
ALTER TABLE maidan_approval_gates
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'request'
    CHECK (kind IN ('request', 'review_round_cap'));
