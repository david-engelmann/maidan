-- Discriminate projector vs result-delivery rows on the egress outbox
-- (Cluster 379.4; see postgres/0086). Existing rows are projector traffic.
ALTER TABLE maidan_egress_outbox
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'projector';
