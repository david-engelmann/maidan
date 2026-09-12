-- Cluster 376 (Wave 2 #23, G6/G-dev-3/W3): a per-workspace spawn budget. Caps
-- how much an agent family may fan out — max child threads per parent, max
-- nesting depth, and max tool calls per thread — well below GitHub's 100/8.
-- Coordination cost is n(n-1)/2 (Brooks/Amdahl/Two-Pizza); the budget refuses a
-- spawn past the cap (SpawnRejected + a ThreadSpawnDenied event). Absent row or
-- NULL column = unlimited on that axis (opt-in, like the WIP limit).
CREATE TABLE IF NOT EXISTS maidan_spawn_budgets (
    workspace_id UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    max_children BIGINT,
    max_depth    BIGINT,
    max_tools    BIGINT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
