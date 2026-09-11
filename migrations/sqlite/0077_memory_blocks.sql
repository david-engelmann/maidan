-- Cluster 373 (Wave 2 #21, H11): attachable labeled memory as room objects
-- (SQLite twin of pg 0078). A memory block is a Letta-shaped
-- {label, description, limit, read_only, value} workspace object; threads attach
-- to it so a parent can watch a child's block. set_value is a full rewrite
-- (last-writer-wins). UNIQUE(workspace_id, label) makes create-if-absent safe.
CREATE TABLE IF NOT EXISTS maidan_memory_blocks (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    label        TEXT NOT NULL,
    description  TEXT,
    char_limit   INTEGER,
    read_only    INTEGER NOT NULL DEFAULT 0,
    value        TEXT NOT NULL DEFAULT '',
    owner_id     TEXT NOT NULL REFERENCES maidan_members(id),
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (workspace_id, label)
);

CREATE TABLE IF NOT EXISTS maidan_thread_memory_blocks (
    thread_id  TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    block_id   TEXT NOT NULL REFERENCES maidan_memory_blocks(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, block_id)
);
CREATE INDEX IF NOT EXISTS idx_thread_memory_blocks_block
    ON maidan_thread_memory_blocks (block_id);
