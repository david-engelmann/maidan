-- Per-member inbox read cursor (Cluster 40.0).

CREATE TABLE maidan_inbox_cursor (
    member_id     UUID NOT NULL PRIMARY KEY REFERENCES maidan_members (id) ON DELETE CASCADE,
    last_read_at  TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01T00:00:00Z',
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
