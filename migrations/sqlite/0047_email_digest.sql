-- Cluster 254 (mirror of postgres 0048): the digest data model.
--
-- maidan_member_delivery_prefs: per-member email delivery mode. Absent row =
-- 'immediate' (Cluster-249 behaviour); 'digest' = periodic rollup instead of
-- per-notification emails (mutually exclusive, enforced by the router).
CREATE TABLE IF NOT EXISTS maidan_member_delivery_prefs (
    member_id TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    mode TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- maidan_member_digest_state: the digest watermark, advanced on each digest send;
-- absent row = "never digested" (count everything).
CREATE TABLE IF NOT EXISTS maidan_member_digest_state (
    member_id TEXT PRIMARY KEY REFERENCES maidan_members(id) ON DELETE CASCADE,
    last_digest_at TEXT NOT NULL DEFAULT (datetime('now'))
);
