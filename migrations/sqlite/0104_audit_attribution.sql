-- Who an audited action was performed for, and under which delegation grant.
-- See the Postgres twin for the reasoning, including why `grant_id` has no
-- foreign key.
ALTER TABLE maidan_audit ADD COLUMN subject_id TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;
ALTER TABLE maidan_audit ADD COLUMN grant_id TEXT;

CREATE INDEX idx_audit_subject ON maidan_audit (subject_id);
CREATE INDEX idx_audit_grant ON maidan_audit (grant_id) WHERE grant_id IS NOT NULL;
