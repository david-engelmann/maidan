-- One live grant per (client, member, workspace, scope). A reused grant is
-- revoked, not duplicated: concurrent consent approvals for the same tuple
-- converge on one row instead of racing the find-then-create in
-- decide_consent.
CREATE UNIQUE INDEX idx_oauth_grants_live_unique
    ON oauth_grants (client_id, member_id, workspace_id, scope)
    WHERE revoked_at IS NULL;
