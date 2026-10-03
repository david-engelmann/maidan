-- Notification retention deletes the oldest read, unsnoozed rows a batch at a
-- time; see the Postgres twin. The prune compares `julianday(created_at)`, so
-- the index is on that expression, or SQLite would not use it.
CREATE INDEX IF NOT EXISTS idx_notifications_prunable
    ON maidan_notifications (julianday(created_at), id)
    WHERE read_at IS NOT NULL AND snoozed_until IS NULL;
