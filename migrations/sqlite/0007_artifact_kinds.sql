-- Maidan artifact kind constraint, v0007, SQLite dialect.
--
-- SQLite cannot add a CHECK to an existing table in place; recreate
-- `maidan_artifacts` with the allowed kind set.

PRAGMA foreign_keys = OFF;

CREATE TABLE maidan_artifacts_new (
    id            TEXT PRIMARY KEY,
    sha256        TEXT NOT NULL UNIQUE,
    size_bytes    INTEGER NOT NULL,
    mime_type     TEXT,
    kind          TEXT NOT NULL
        CHECK (kind IN (
            'screenshot',
            'recording',
            'transcript',
            'code_dump',
            'attachment'
        )),
    uploaded_by   TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    tombstoned_at TEXT
);

INSERT INTO maidan_artifacts_new
SELECT id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at
FROM maidan_artifacts;

DROP TABLE maidan_artifacts;

ALTER TABLE maidan_artifacts_new RENAME TO maidan_artifacts;

CREATE INDEX idx_artifacts_uploader ON maidan_artifacts (uploaded_by);

PRAGMA foreign_keys = ON;
