-- Who blessed each egress allowlist entry, so the console's allowlist can say
-- who added a destination beside when. The audit trail already records it,
-- but only as far back as an audit page reaches; the entry itself must answer.
--
-- Entries blessed before this column have no blesser; deleting the member
-- leaves the entry in force with no blesser, since the blessing is the
-- workspace's, not the person's.
ALTER TABLE maidan_egress_targets
    ADD COLUMN created_by UUID REFERENCES maidan_members(id) ON DELETE SET NULL;
