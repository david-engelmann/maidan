-- Cluster 241 (mirror of postgres 0044): per-member notification preferences — one
-- row per (member, EventKind) with a `muted` flag (absence = notify). The
-- zero-blast-radius foundation for preference-aware routing; no router change yet.
CREATE TABLE IF NOT EXISTS maidan_notification_prefs (
    member_id TEXT NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    muted INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (member_id, kind)
);
