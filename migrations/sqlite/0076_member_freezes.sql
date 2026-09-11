-- Cluster 372 (Wave 2 #20, G17/B25): a freeze-member kill-switch (SQLite twin of
-- pg 0077). A row freezes the member — `claim_next` refuses them and freezing
-- drops their active leases. The freeze is the gate; an operator unfreezes by
-- deleting the row. NOT G4 PAUSE.
CREATE TABLE IF NOT EXISTS maidan_member_freezes (
    member_id  TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    frozen_at  TEXT NOT NULL,
    frozen_by  TEXT NOT NULL REFERENCES maidan_members(id),
    reason     TEXT
);
