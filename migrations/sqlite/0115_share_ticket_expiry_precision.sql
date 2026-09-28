-- A share ticket's expiry is checked at full precision, as in Postgres.
-- The old CHECK compared `datetime()` values, which drop the fractional
-- second, against a `created_at` the database stamped after the store had
-- validated the expiry. An expiry in the same wall-clock second as the insert
-- passed validation and then failed the CHECK, so minting the ticket was a
-- 500. The store now writes `created_at` from the instant it validated
-- against, and `julianday()` keeps the milliseconds. Postgres needs no twin:
-- its CHECK compares `timestamptz` values.
--
-- SQLite cannot alter a CHECK, so the table is rebuilt. Dropping it would
-- cascade to `maidan_share_ticket_artifacts`, so those rows wait in a table
-- without foreign keys and go back once the new table is in place.
CREATE TABLE maidan_share_ticket_artifacts_hold (
    ticket_id TEXT NOT NULL,
    sha256 TEXT NOT NULL
);
INSERT INTO maidan_share_ticket_artifacts_hold (ticket_id, sha256)
    SELECT ticket_id, sha256 FROM maidan_share_ticket_artifacts;
DROP TABLE maidan_share_ticket_artifacts;

CREATE TABLE maidan_share_tickets_rebuilt (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES maidan_channels(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES maidan_members(id),
    created_by TEXT NOT NULL REFERENCES maidan_members(id),
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (julianday(expires_at) > julianday(created_at))
);
INSERT INTO maidan_share_tickets_rebuilt
    (id, workspace_id, channel_id, owner_id, created_by, token_hash, expires_at, revoked_at, created_at)
SELECT id, workspace_id, channel_id, owner_id, created_by, token_hash, expires_at, revoked_at, created_at
FROM maidan_share_tickets;
DROP TABLE maidan_share_tickets;
ALTER TABLE maidan_share_tickets_rebuilt RENAME TO maidan_share_tickets;
CREATE INDEX idx_share_tickets_workspace_created
    ON maidan_share_tickets (workspace_id, created_at DESC);

CREATE TABLE maidan_share_ticket_artifacts (
    ticket_id TEXT NOT NULL REFERENCES maidan_share_tickets(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL REFERENCES maidan_artifacts(sha256) ON DELETE CASCADE,
    PRIMARY KEY (ticket_id, sha256)
);
INSERT INTO maidan_share_ticket_artifacts (ticket_id, sha256)
    SELECT ticket_id, sha256 FROM maidan_share_ticket_artifacts_hold;
DROP TABLE maidan_share_ticket_artifacts_hold;
