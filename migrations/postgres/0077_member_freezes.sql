-- Cluster 372 (Wave 2 #20, G17/B25): a freeze-member kill-switch. Presence of a
-- row freezes the member — `claim_next` refuses them and freezing drops their
-- active leases (releases their claimed threads). A frozen member stays frozen
-- until an operator explicitly unfreezes (deletes the row): the freeze is the
-- gate. NOT G4 PAUSE (that pauses a thread/workspace); this stops one member.
CREATE TABLE IF NOT EXISTS maidan_member_freezes (
    member_id  UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    frozen_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    frozen_by  UUID NOT NULL REFERENCES maidan_members(id),
    reason     TEXT
);
