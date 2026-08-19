-- Cluster 241 (Program C, Arc H): per-member notification preferences. One row per
-- (member, EventKind) with a `muted` flag — the notification router (Cluster 238)
-- skips writing a notification when the recipient has muted that kind. Absence of a
-- row = the default (notify). The zero-blast-radius foundation (Cluster 230 pattern)
-- for preference-aware routing — no router change or routes in this cluster.
CREATE TABLE IF NOT EXISTS maidan_notification_prefs (
    member_id UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    muted BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (member_id, kind)
);
