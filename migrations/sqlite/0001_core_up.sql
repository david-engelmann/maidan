-- Maidan core schema, v0001, SQLite dialect.
--
-- Mirrors the Postgres dialect 1:1. Differences:
-- - UUIDs are stored as TEXT (36-char canonical form).
-- - Timestamps are stored as TEXT in RFC3339 / ISO-8601 form,
--   provided explicitly by the application layer.
-- - JSON columns are TEXT validated by the JSON1 extension when needed.
-- - audit.id is INTEGER PRIMARY KEY AUTOINCREMENT instead of BIGSERIAL.
-- - No CREATE EXTENSION (SQLite has no extensions in this dialect).

CREATE TABLE maidan_workspaces (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    tombstoned_at TEXT
);

CREATE TABLE maidan_members (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    handle        TEXT NOT NULL,
    display_name  TEXT,
    kind          TEXT NOT NULL CHECK (kind IN ('human', 'agent')),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    tombstoned_at TEXT,
    UNIQUE (workspace_id, handle)
);

CREATE INDEX idx_members_workspace ON maidan_members (workspace_id);

CREATE TABLE maidan_channels (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    topic         TEXT,
    private       INTEGER NOT NULL DEFAULT 0 CHECK (private IN (0, 1)),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    tombstoned_at TEXT,
    UNIQUE (workspace_id, name)
);

CREATE INDEX idx_channels_workspace ON maidan_channels (workspace_id);

CREATE TABLE maidan_threads (
    id            TEXT PRIMARY KEY,
    channel_id    TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    title         TEXT,
    state         TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'closed', 'archived')),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    tombstoned_at TEXT
);

CREATE INDEX idx_threads_channel ON maidan_threads (channel_id);

CREATE TABLE maidan_artifacts (
    id            TEXT PRIMARY KEY,
    sha256        TEXT NOT NULL UNIQUE,
    size_bytes    INTEGER NOT NULL,
    mime_type     TEXT,
    kind          TEXT NOT NULL,
    uploaded_by   TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    tombstoned_at TEXT
);

CREATE INDEX idx_artifacts_uploader ON maidan_artifacts (uploaded_by);

CREATE TABLE maidan_messages (
    id            TEXT PRIMARY KEY,
    thread_id     TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    author_id     TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE RESTRICT,
    body          TEXT NOT NULL,
    metadata      TEXT NOT NULL DEFAULT '{}',
    posted_at     TEXT NOT NULL,
    edited_at     TEXT,
    tombstoned_at TEXT
);

CREATE INDEX idx_messages_thread_posted ON maidan_messages (thread_id, posted_at);
CREATE INDEX idx_messages_author ON maidan_messages (author_id);

CREATE TABLE maidan_mentions (
    message_id TEXT NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    member_id  TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, member_id)
);

CREATE INDEX idx_mentions_member ON maidan_mentions (member_id);

CREATE TABLE maidan_votes (
    message_id TEXT NOT NULL REFERENCES maidan_messages(id) ON DELETE CASCADE,
    member_id  TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, member_id, kind)
);

CREATE INDEX idx_votes_member ON maidan_votes (member_id);

CREATE TABLE maidan_references (
    id         TEXT PRIMARY KEY,
    src_kind   TEXT NOT NULL CHECK (src_kind IN ('thread', 'message')),
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL CHECK (dst_kind IN ('thread', 'message')),
    dst_id     TEXT NOT NULL,
    relation   TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (src_kind, src_id, dst_kind, dst_id, relation)
);

CREATE INDEX idx_references_src ON maidan_references (src_kind, src_id);
CREATE INDEX idx_references_dst ON maidan_references (dst_kind, dst_id);

CREATE TABLE maidan_audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    actor_id    TEXT REFERENCES maidan_members(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    target_kind TEXT,
    target_id   TEXT,
    metadata    TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_audit_occurred ON maidan_audit (occurred_at DESC);
CREATE INDEX idx_audit_actor ON maidan_audit (actor_id);
