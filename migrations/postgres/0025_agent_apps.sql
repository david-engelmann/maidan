-- Installed apps (Cluster 57.0): workspace-scoped apps with bot members and app tokens.

CREATE TABLE maidan_apps (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    slug            TEXT NOT NULL,
    name            TEXT NOT NULL,
    description     TEXT,
    created_by      UUID NOT NULL REFERENCES maidan_members (id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, slug)
);

CREATE INDEX idx_apps_workspace ON maidan_apps (workspace_id);

CREATE TABLE maidan_app_installations (
    id                    UUID PRIMARY KEY,
    app_id                UUID NOT NULL REFERENCES maidan_apps (id) ON DELETE CASCADE,
    workspace_id          UUID NOT NULL REFERENCES maidan_workspaces (id) ON DELETE CASCADE,
    bot_member_id         UUID NOT NULL REFERENCES maidan_members (id) ON DELETE CASCADE,
    granted_capabilities  TEXT NOT NULL,
    installed_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at            TIMESTAMPTZ
);

CREATE INDEX idx_app_installations_workspace ON maidan_app_installations (workspace_id);
CREATE INDEX idx_app_installations_app ON maidan_app_installations (app_id);

ALTER TABLE maidan_api_tokens
    ADD COLUMN app_installation_id UUID REFERENCES maidan_app_installations (id) ON DELETE CASCADE;

CREATE INDEX idx_api_tokens_app_installation ON maidan_api_tokens (app_installation_id)
    WHERE app_installation_id IS NOT NULL;
