-- Cluster 376 (Wave 2 #23, G6/G-dev-3/W3): a per-workspace spawn budget (SQLite
-- twin of pg 0080). Caps agent fan-out — max children per parent, max nesting
-- depth, max tool calls per thread. Absent row or NULL column = unlimited.
CREATE TABLE IF NOT EXISTS maidan_spawn_budgets (
    workspace_id TEXT PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    max_children INTEGER,
    max_depth    INTEGER,
    max_tools    INTEGER,
    updated_at   TEXT NOT NULL
);
