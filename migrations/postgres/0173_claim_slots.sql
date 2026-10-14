-- Claim slots (maidan_types::footprint, the `slot` kind). A footprint item
-- of kind `slot` names a shared resource, such as a build slot. Slots are
-- counted across a workspace; with a capacity set here, the claim queue
-- defers a thread whose slot already has that many live claims.
ALTER TABLE maidan_thread_footprints DROP CONSTRAINT maidan_thread_footprints_kind_check;
ALTER TABLE maidan_thread_footprints ADD CONSTRAINT maidan_thread_footprints_kind_check
    CHECK (kind IN ('path', 'glob', 'crate', 'lane', 'branch', 'slot'));
-- Slot counts read across repositories.
CREATE INDEX idx_thread_footprints_slot
    ON maidan_thread_footprints (workspace_id, match_namespace, match_key);

CREATE TABLE maidan_slot_capacities (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    slot         TEXT NOT NULL,
    capacity     BIGINT NOT NULL CHECK (capacity > 0),
    updated_at   TIMESTAMPTZ NOT NULL,
    updated_by   UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    PRIMARY KEY (workspace_id, slot)
);
