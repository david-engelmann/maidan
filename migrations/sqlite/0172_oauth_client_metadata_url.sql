-- A client identified by a client metadata document gets a row in
-- oauth_clients on first use, so its authorization codes and grants
-- satisfy their foreign keys (deleting the row still cascades to them).
-- metadata_url marks the row as document-backed: it records where the
-- document lives, and resolution never treats a marked row as
-- pre-registered. NULL for pre-registered clients.
ALTER TABLE oauth_clients ADD COLUMN metadata_url TEXT;
