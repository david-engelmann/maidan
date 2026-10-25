-- Whether a change entering review in the workspace spawns its
-- second-family `kind: review` child (S4). 'spawn' is the behaviour before
-- this column; 'off' leaves review with the pull request's own reviewers.
ALTER TABLE maidan_workspaces
    ADD COLUMN review_children TEXT NOT NULL DEFAULT 'spawn'
        CHECK (review_children IN ('spawn', 'off'));
