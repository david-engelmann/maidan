-- Cluster 238 (Program C, Arc G): dedup guard for the notification router — one
-- notification per (recipient, source event). Every server replica runs the
-- router's bus consumer, so the same event reaches each; `create_notification_if_absent`
-- inserts ON CONFLICT DO NOTHING against this unique index, so a replayed event
-- or a second replica cannot double-notify.
CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_dedup
    ON maidan_notifications (member_id, source_log_id);
