-- Persisted, single-use, TTL'd OAuth authorization codes (Cluster 104.0.1).
-- SQLite mirror of the Postgres schema; only the code hash is stored.
CREATE TABLE maidan_oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    app_id         TEXT NOT NULL REFERENCES maidan_apps(id) ON DELETE CASCADE,
    workspace_id   TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    redirect_uri   TEXT NOT NULL,
    code_challenge TEXT,
    expires_at     TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_oauth_codes_expires_at ON maidan_oauth_codes (expires_at);
