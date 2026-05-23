-- Maidan search schema, v0002, SQLite dialect.
--
-- SQLite has no tsvector. Use an FTS5 virtual table keyed by an
-- autoincrement rowid that maps back to the canonical message UUID via
-- `maidan_messages_fts_map`. Triggers on `maidan_messages` keep the
-- FTS5 contents in sync.

CREATE TABLE maidan_messages_fts_map (
    rowid      INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL UNIQUE REFERENCES maidan_messages(id) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE maidan_messages_fts USING fts5(
    body,
    topic,
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER maidan_messages_fts_insert
AFTER INSERT ON maidan_messages
WHEN NEW.tombstoned_at IS NULL
BEGIN
    INSERT INTO maidan_messages_fts_map (message_id) VALUES (NEW.id);
    INSERT INTO maidan_messages_fts (rowid, body, topic)
    VALUES (
        (SELECT rowid FROM maidan_messages_fts_map WHERE message_id = NEW.id),
        NEW.body,
        coalesce(json_extract(NEW.metadata, '$.topic'), '')
    );
END;

CREATE TRIGGER maidan_messages_fts_tombstone
AFTER UPDATE OF tombstoned_at ON maidan_messages
WHEN NEW.tombstoned_at IS NOT NULL AND OLD.tombstoned_at IS NULL
BEGIN
    DELETE FROM maidan_messages_fts
    WHERE rowid = (SELECT rowid FROM maidan_messages_fts_map WHERE message_id = OLD.id);
    DELETE FROM maidan_messages_fts_map WHERE message_id = OLD.id;
END;

CREATE TRIGGER maidan_messages_fts_update
AFTER UPDATE OF body, metadata ON maidan_messages
WHEN NEW.tombstoned_at IS NULL
BEGIN
    UPDATE maidan_messages_fts
    SET body = NEW.body,
        topic = coalesce(json_extract(NEW.metadata, '$.topic'), '')
    WHERE rowid = (SELECT rowid FROM maidan_messages_fts_map WHERE message_id = NEW.id);
END;
