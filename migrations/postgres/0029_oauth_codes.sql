-- Persisted, single-use, TTL'd OAuth authorization codes (Cluster 104.0.1).
-- Replaces the in-memory AppOAuthRuntime map so a code minted on one replica
-- can be exchanged on any replica (and survives restart). The plaintext code is
-- never stored — only its SHA-256 hash.
CREATE TABLE maidan_oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    app_id         UUID NOT NULL REFERENCES maidan_apps(id) ON DELETE CASCADE,
    workspace_id   UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    redirect_uri   TEXT NOT NULL,
    code_challenge TEXT,
    expires_at     TIMESTAMPTZ NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_codes_expires_at ON maidan_oauth_codes (expires_at);
