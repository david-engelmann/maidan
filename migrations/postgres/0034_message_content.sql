-- Cluster 173: typed structured message content. Additive + nullable — `body`
-- remains the canonical searchable plain-text projection, so the 0002 search
-- tsvector/FTS are untouched. NULL = plain/legacy message.
ALTER TABLE maidan_messages ADD COLUMN content JSONB;
