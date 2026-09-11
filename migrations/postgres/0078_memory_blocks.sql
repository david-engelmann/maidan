-- Cluster 373 (Wave 2 #21, H11): attachable labeled memory as room objects. A
-- memory block is a Letta-shaped {label, description, limit, read_only, value}
-- workspace object; threads attach to it so a parent can watch a child's block
-- (shared mutable state, NOT a transcript, NOT RAG). set_value is a full rewrite
-- (last-writer-wins). UNIQUE(workspace_id, label) makes create-if-absent safe
-- under concurrency (INSERT ... ON CONFLICT DO NOTHING).
CREATE TABLE IF NOT EXISTS maidan_memory_blocks (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    label        TEXT NOT NULL,
    description  TEXT,
    char_limit   BIGINT,
    read_only    BOOLEAN NOT NULL DEFAULT FALSE,
    value        TEXT NOT NULL DEFAULT '',
    owner_id     UUID NOT NULL REFERENCES maidan_members(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, label)
);

-- Attachments: which blocks a thread ("room") carries. A parent watches a
-- child's result block by sharing the block, no nested runtime.
CREATE TABLE IF NOT EXISTS maidan_thread_memory_blocks (
    thread_id  UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    block_id   UUID NOT NULL REFERENCES maidan_memory_blocks(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, block_id)
);
CREATE INDEX IF NOT EXISTS idx_thread_memory_blocks_block
    ON maidan_thread_memory_blocks (block_id);
