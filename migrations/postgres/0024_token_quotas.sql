-- Per-token capability quotas (Cluster 54.0).

CREATE TABLE maidan_token_quotas (
    token_id       UUID NOT NULL REFERENCES maidan_api_tokens(id) ON DELETE CASCADE,
    capability     TEXT NOT NULL,
    max_per_window INTEGER NOT NULL CHECK (max_per_window > 0),
    window_secs    INTEGER NOT NULL CHECK (window_secs > 0),
    PRIMARY KEY (token_id, capability)
);

CREATE INDEX idx_token_quotas_token ON maidan_token_quotas (token_id);
