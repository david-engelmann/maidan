-- Cluster 355 (W1): a durable OWNER axis on threads, distinct from the assignee
-- (the claimer who does the work). The owner is the accountable party — a human,
-- typically — who governs the task: they receive stuck notifications and, once
-- set, opt the thread into separation-of-duties (the claimer cannot land its own
-- work). Orthogonal to both the ThreadState FSM and the assignee/claim axis.
-- ON DELETE SET NULL so a member's removal clears ownership rather than blocking
-- the delete (mirrors assignee_id, Cluster 171).
ALTER TABLE maidan_threads
    ADD COLUMN owner_id UUID REFERENCES maidan_members(id) ON DELETE SET NULL;

CREATE INDEX idx_threads_owner ON maidan_threads (owner_id);
