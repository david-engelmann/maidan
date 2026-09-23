-- A workspace's ceiling on how long a delegation grant may live (D-B,
-- 2026-09-23). A grant is the standing authority to keep minting short-lived
-- tokens, so its lifetime is the real exposure; no row means the default,
-- 90 days. Set with token:admin.
CREATE TABLE maidan_delegation_policies (
    workspace_id   UUID PRIMARY KEY REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    max_grant_days BIGINT NOT NULL CHECK (max_grant_days BETWEEN 1 AND 3650),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
