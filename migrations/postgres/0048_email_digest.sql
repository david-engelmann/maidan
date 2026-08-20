-- Cluster 254 (Program C, Arc I): the digest data model. Two per-member tables.
--
-- maidan_member_delivery_prefs: how a member wants notification emails delivered.
-- Absent row = 'immediate' (the Cluster-249 behaviour). 'digest' opts out of
-- per-notification emails in favour of a periodic rollup — the two are mutually
-- exclusive, enforced by the router (255) skipping digest-mode members.
CREATE TABLE IF NOT EXISTS maidan_member_delivery_prefs (
    member_id UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    mode TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- maidan_member_digest_state: the digest watermark. `last_digest_at` is advanced
-- each time the sweeper emails a digest, so the next run only counts notifications
-- created since. An absent row means "never digested" — count everything.
CREATE TABLE IF NOT EXISTS maidan_member_digest_state (
    member_id UUID PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    last_digest_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
