-- Who an audited action was performed for, and under which delegation grant.
-- `actor_id` already records who acted; for a delegated action it is the
-- delegate, and these record the member it acted for and the grant it used.
-- A direct action has subject = actor and no grant.
--
-- `grant_id` carries no foreign key on purpose: an audit row must keep naming
-- the authority an action was taken under, whatever later happens to the grant.
ALTER TABLE maidan_audit
    ADD COLUMN subject_id UUID REFERENCES maidan_members(id) ON DELETE SET NULL,
    ADD COLUMN grant_id UUID;

CREATE INDEX idx_audit_subject ON maidan_audit (subject_id);
CREATE INDEX idx_audit_grant ON maidan_audit (grant_id) WHERE grant_id IS NOT NULL;
