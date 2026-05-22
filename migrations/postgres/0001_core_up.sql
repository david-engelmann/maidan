-- Maidan core schema, v0001.
--
-- All tables are prefixed `maidan_` so the schema can coexist with other
-- applications inside a shared database. Logical deletes are recorded via
-- a `tombstoned_at` timestamp; physical deletion is reserved for GDPR
-- right-of-erasure flows that arrive in a later cluster.

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE maidan_workspaces (
    id            UUID PRIMARY KEY,
    name          TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tombstoned_at TIMESTAMPTZ
);

CREATE TABLE maidan_members (
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    handle        TEXT NOT NULL,
    display_name  TEXT,
    kind          TEXT NOT NULL CHECK (kind IN ('human', 'agent')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tombstoned_at TIMESTAMPTZ,
    UNIQUE (workspace_id, handle)
);

CREATE INDEX idx_members_workspace ON maidan_members (workspace_id);

CREATE TABLE maidan_channels (
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    topic         TEXT,
    private       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tombstoned_at TIMESTAMPTZ,
    UNIQUE (workspace_id, name)
);

CREATE INDEX idx_channels_workspace ON maidan_channels (workspace_id);

CREATE TABLE maidan_threads (
    id            UUID PRIMARY KEY,
    channel_id    UUID NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    title         TEXT,
    state         TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'closed', 'archived')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tombstoned_at TIMESTAMPTZ
);

CREATE INDEX idx_threads_channel ON maidan_threads (channel_id);

CREATE TABLE maidan_artifacts (
    id            UUID PRIMARY KEY,
    sha256        TEXT NOT NULL UNIQUE,
    size_bytes    BIGINT NOT NULL,
    mime_type     TEXT,
    kind          TEXT NOT NULL,
    uploaded_by   UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tombstoned_at TIMESTAMPTZ
);

CREATE INDEX idx_artifacts_uploader ON maidan_artifacts (uploaded_by);

CREATE TABLE maidan_messages (
    id            UUID PRIMARY KEY,
    thread_id     UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    author_id     UUID NOT NULL REFERENCES maidan_members(id) ON DELETE RESTRICT,
    body          TEXT NOT NULL,
    metadata      JSONB NOT NULL DEFAULT '{}'::jsonb,
    posted_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    edited_at     TIMESTAMPTZ,
    tombstoned_at TIMESTAMPTZ
);

CREATE INDEX idx_messages_thread_posted ON maidan_messages (thread_id, posted_at);
CREATE INDEX idx_messages_author ON maidan_messages (author_id);

CREATE TABLE maidan_mentions (
    message_id UUID NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (message_id, member_id)
);

CREATE INDEX idx_mentions_member ON maidan_mentions (member_id);

CREATE TABLE maidan_votes (
    message_id UUID NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (message_id, member_id, kind)
);

CREATE INDEX idx_votes_member ON maidan_votes (member_id);

CREATE TABLE maidan_references (
    id         UUID PRIMARY KEY,
    src_kind   TEXT NOT NULL CHECK (src_kind IN ('thread', 'message')),
    src_id     UUID NOT NULL,
    dst_kind   TEXT NOT NULL CHECK (dst_kind IN ('thread', 'message')),
    dst_id     UUID NOT NULL,
    relation   TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (src_kind, src_id, dst_kind, dst_id, relation)
);

CREATE INDEX idx_references_src ON maidan_references (src_kind, src_id);
CREATE INDEX idx_references_dst ON maidan_references (dst_kind, dst_id);

CREATE TABLE maidan_audit (
    id          BIGSERIAL PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor_id    UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    target_kind TEXT,
    target_id   UUID,
    metadata    JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX idx_audit_occurred ON maidan_audit (occurred_at DESC);
CREATE INDEX idx_audit_actor ON maidan_audit (actor_id);
