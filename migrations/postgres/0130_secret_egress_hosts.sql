-- The hosts each workspace trusts to receive its secret values (Open Work
-- Next #22). On egress (webhooks, automation HTTP, A2A push) the SecretBroker
-- substitutes a `secret://<name>` ref from the sending workspace's secrets only
-- when the target URL's host is listed here for that workspace; any other host
-- gets the literal ref. It replaces one instance-wide environment list, which
-- let every tenant's secrets go to any host one operator trusted.
--
-- Empty ⇒ substitute nowhere. `host` is lowercase with no scheme or port, the
-- form the egress URL's host parses to (`normalize_secret_egress_host`).
CREATE TABLE IF NOT EXISTS maidan_secret_egress_hosts (
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, host)
);
