-- RFC 7591 dynamic client registration (`docs/OAuth.md`, decided 2026-11-05).
-- A client that registered itself through POST /oauth/register is a row in
-- oauth_clients like any other, so its codes and grants keep their foreign
-- keys and deleting it cascades to them. registered_at marks it as
-- self-registered (NULL for pre-registered and metadata-document clients),
-- and the two later columns are what the sweep reads: a client that never
-- completes an authorization within a day, or goes unused for 90 days, is
-- deleted.
ALTER TABLE oauth_clients ADD COLUMN registered_at TIMESTAMPTZ;
-- The first token exchange this client completed. NULL until then.
ALTER TABLE oauth_clients ADD COLUMN authorized_at TIMESTAMPTZ;
-- The last token exchange or refresh. Set at registration.
ALTER TABLE oauth_clients ADD COLUMN last_used_at TIMESTAMPTZ;
CREATE INDEX idx_oauth_clients_registered ON oauth_clients (registered_at)
    WHERE registered_at IS NOT NULL;
