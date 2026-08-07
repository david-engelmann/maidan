-- Cluster 171: task assignment / handoff. An assignee axis on threads,
-- orthogonal to the ThreadState FSM. ON DELETE SET NULL so a member's removal
-- unassigns their threads rather than blocking the delete.
ALTER TABLE maidan_threads
    ADD COLUMN assignee_id TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;

CREATE INDEX idx_threads_assignee ON maidan_threads (assignee_id);
