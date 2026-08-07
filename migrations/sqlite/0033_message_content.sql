-- Cluster 173: typed structured message content. Additive + nullable — `body`
-- remains the canonical searchable plain-text projection, so the 0002 search
-- FTS triggers are untouched. NULL = plain/legacy message. Stored as a JSON
-- string (like `metadata`).
ALTER TABLE maidan_messages ADD COLUMN content TEXT;
