-- A reap's claim on an orphaned artifact's bytes while it deletes them. See
-- the Postgres twin. `expires_at` is millisecond `...Z` text, compared as a
-- string against `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`.
CREATE TABLE IF NOT EXISTS maidan_artifact_reaps (
    sha256 TEXT PRIMARY KEY,
    token TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
