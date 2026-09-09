-- Cluster 359 (N5, mirror of postgres 0065): notification snooze. NULL = not
-- snoozed; a future value hides the notification from the default inbox + badge
-- until it lapses.
ALTER TABLE maidan_notifications ADD COLUMN snoozed_until TEXT;
