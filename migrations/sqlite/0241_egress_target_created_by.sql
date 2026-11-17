-- Who blessed each egress allowlist entry; see the Postgres twin (0241).
ALTER TABLE maidan_egress_targets
    ADD COLUMN created_by TEXT REFERENCES maidan_members(id) ON DELETE SET NULL;
