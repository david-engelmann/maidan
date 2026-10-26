-- Next 38 C10 (mirror of postgres 0210): widen the artifact-kind CHECK to allow
-- 'digest'. SQLite cannot alter a CHECK in place, so recreate the table with
-- the columns it has now (0054's, plus `filename` from 0128, less
-- `tombstoned_at` dropped by 0136). The migration runs in a transaction, where
-- `PRAGMA foreign_keys` is a no-op, so dropping the table would cascade into
-- share-ticket artifact rows; hold them aside and restore them, as 0115 does.
CREATE TABLE maidan_share_ticket_artifacts_hold AS
    SELECT ticket_id, sha256 FROM maidan_share_ticket_artifacts;
DROP TABLE maidan_share_ticket_artifacts;

CREATE TABLE maidan_artifacts_new (
    id          TEXT PRIMARY KEY,
    sha256      TEXT NOT NULL UNIQUE,
    size_bytes  INTEGER NOT NULL,
    mime_type   TEXT,
    kind        TEXT NOT NULL
        CHECK (kind IN (
            'screenshot',
            'recording',
            'transcript',
            'code_dump',
            'attachment',
            'context_snapshot',
            'digest'
        )),
    uploaded_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL,
    filename    TEXT
);

INSERT INTO maidan_artifacts_new (id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, filename)
SELECT id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, filename
FROM maidan_artifacts;

DROP TABLE maidan_artifacts;

ALTER TABLE maidan_artifacts_new RENAME TO maidan_artifacts;

CREATE INDEX idx_artifacts_uploader ON maidan_artifacts (uploaded_by);

CREATE TABLE maidan_share_ticket_artifacts (
    ticket_id TEXT NOT NULL REFERENCES maidan_share_tickets(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL REFERENCES maidan_artifacts(sha256) ON DELETE CASCADE,
    PRIMARY KEY (ticket_id, sha256)
);
INSERT INTO maidan_share_ticket_artifacts (ticket_id, sha256)
    SELECT ticket_id, sha256 FROM maidan_share_ticket_artifacts_hold;
DROP TABLE maidan_share_ticket_artifacts_hold;
