-- Notification retention deletes the oldest read, unsnoozed rows a batch at a
-- time. Without an index on their age, every batch scanned and sorted the
-- whole table, so a first sweep cost grew with the square of its size. The
-- partial index holds only the rows retention may delete, in the order it
-- deletes them, so a batch reads what it deletes and not the unread rows an
-- agent never marks.
CREATE INDEX IF NOT EXISTS idx_notifications_prunable
    ON maidan_notifications (created_at, id)
    WHERE read_at IS NOT NULL AND snoozed_until IS NULL;
