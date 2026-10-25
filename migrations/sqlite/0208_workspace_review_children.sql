-- A workspace's review-children setting; see the Postgres twin (0208).
ALTER TABLE maidan_workspaces
    ADD COLUMN review_children TEXT NOT NULL DEFAULT 'spawn'
        CHECK (review_children IN ('spawn', 'off'));
