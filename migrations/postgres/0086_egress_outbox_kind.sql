-- Discriminate projector vs result-delivery rows on the egress outbox
-- (Cluster 379.4). Update-in-place keys on the result-delivery row's
-- `external_ref`; without a kind, a projector `MessagePosted` aimed at the
-- same GitHub issue would PATCH the result comment. Existing rows are
-- projector traffic (the only kind that existed before this column).
ALTER TABLE maidan_egress_outbox
    ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'projector';
