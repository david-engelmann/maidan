-- Cluster 359 (N5): notification snooze. A member defers a notification to
-- reappear later — while `snoozed_until` is in the future the notification drops
-- out of the default inbox list and the unread-count badge, then resurfaces
-- automatically once it lapses. NULL = not snoozed. Orthogonal to `read_at`.
ALTER TABLE maidan_notifications ADD COLUMN snoozed_until TIMESTAMPTZ;
