-- RFC 7591 dynamic client registration (`docs/OAuth.md`, decided 2026-11-05).
-- SQLite dialect. A self-registered client is a row in oauth_clients like
-- any other; registered_at marks it (NULL for pre-registered and
-- metadata-document clients), and the sweep reads authorized_at and
-- last_used_at to delete one never authorized within a day or unused for
-- 90 days.
ALTER TABLE oauth_clients ADD COLUMN registered_at TEXT;
ALTER TABLE oauth_clients ADD COLUMN authorized_at TEXT;
ALTER TABLE oauth_clients ADD COLUMN last_used_at TEXT;
CREATE INDEX idx_oauth_clients_registered ON oauth_clients (registered_at)
    WHERE registered_at IS NOT NULL;
