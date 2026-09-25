-- An audit row keeps naming who acted and for whom, whatever later happens to
-- them; see the Postgres twin. SQLite cannot drop a foreign key, so the table
-- is rebuilt without the two on `actor_id` and `subject_id`. Nothing
-- references `maidan_audit`.
CREATE TABLE maidan_audit_rebuilt (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    actor_id    TEXT,
    action      TEXT NOT NULL,
    target_kind TEXT,
    target_id   TEXT,
    metadata    TEXT NOT NULL DEFAULT '{}',
    subject_id  TEXT,
    grant_id    TEXT
);
INSERT INTO maidan_audit_rebuilt
    (id, occurred_at, actor_id, action, target_kind, target_id, metadata, subject_id, grant_id)
SELECT id, occurred_at, actor_id, action, target_kind, target_id, metadata, subject_id, grant_id
FROM maidan_audit;
DROP TABLE maidan_audit;
ALTER TABLE maidan_audit_rebuilt RENAME TO maidan_audit;

CREATE INDEX idx_audit_occurred ON maidan_audit (occurred_at DESC);
CREATE INDEX idx_audit_actor ON maidan_audit (actor_id);
CREATE INDEX idx_audit_subject ON maidan_audit (subject_id);
CREATE INDEX idx_audit_grant ON maidan_audit (grant_id) WHERE grant_id IS NOT NULL;
