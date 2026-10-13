-- The notices query (a projector shape filtered by channel and kind) read
-- the event log the slow way: it paged list_events_after over every row in
-- the workspace and filtered in Rust, because the only index was
-- (workspace_id, id). At 100k rows that is ~200 sequential page queries and
-- a decryption per row to collect a few hundred matches. This index lets
-- the filtered read (Store::list_events_matching) seek straight to the
-- matching rows; `id` rides last so each (workspace, channel, kind) range
-- still comes out in log order. On the partitioned parent the index is
-- created on every partition, existing and future. See the SQLite twin.
CREATE INDEX idx_events_workspace_channel_kind
    ON maidan_events (workspace_id, channel_id, kind, id);
