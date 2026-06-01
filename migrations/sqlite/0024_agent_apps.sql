-- Installed apps (Cluster 57.0), SQLite dialect.

CREATE TABLE maidan_apps (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    slug            TEXT NOT NULL,
    name            TEXT NOT NULL,
    description     TEXT,
    created_by      TEXT NOT NULL REFERENCES maidan_members (id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (workspace_id, slug)
);

CREATE INDEX idx_apps_workspace ON maidan_apps (workspace_id);

CREATE TABLE maidan_app_installations (
    id                    TEXT PRIMARY KEY,
    app_id                TEXT NOT NULL REFERENCES maidan_apps (id) ON DELETE CASCADE,
    workspace_id          TEXT NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    bot_member_id         TEXT NOT NULL REFERENCES maidan_members (id) ON DELETE CASCADE,
    granted_capabilities  TEXT NOT NULL,
    installed_at          TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at            TEXT
);

CREATE INDEX idx_app_installations_workspace ON maidan_app_installations (workspace_id);
CREATE INDEX idx_app_installations_app ON maidan_app_installations (app_id);

ALTER TABLE maidan_api_tokens ADD COLUMN app_installation_id TEXT REFERENCES maidan_app_installations (id) ON DELETE CASCADE;

CREATE INDEX idx_api_tokens_app_installation ON maidan_api_tokens (app_installation_id);
