-- Prepaid credits paid in USDC on Base; see the Postgres twin (0231).
CREATE TABLE maidan_credit_ledger (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('deposit', 'charge', 'grant')),
    source        TEXT CHECK (source IN ('usdc_base', 'stripe', 'grant')),
    external_ref  TEXT,
    usd_micros    INTEGER NOT NULL,
    unpaid_micros INTEGER NOT NULL DEFAULT 0,
    meter         TEXT,
    month         TEXT,
    from_units    INTEGER,
    through_units INTEGER,
    chain         TEXT,
    tx_hash       TEXT,
    log_index     INTEGER,
    top_up_id     TEXT,
    note          TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_credit_ledger_workspace ON maidan_credit_ledger (workspace_id, created_at);
CREATE UNIQUE INDEX uq_credit_ledger_source_ref
    ON maidan_credit_ledger (source, external_ref);
CREATE UNIQUE INDEX uq_credit_ledger_charge
    ON maidan_credit_ledger (workspace_id, meter, month, from_units) WHERE kind = 'charge';

CREATE TABLE maidan_credit_top_ups (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    method        TEXT NOT NULL DEFAULT 'usdc' CHECK (method IN ('usdc', 'card')),
    chain         TEXT,
    address       TEXT,
    checkout_session_id TEXT,
    checkout_url  TEXT,
    usd           INTEGER NOT NULL,
    amount_micros INTEGER NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('open', 'paid', 'expired')),
    created_at    TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    paid_at       TEXT
);
CREATE INDEX idx_credit_top_ups_workspace ON maidan_credit_top_ups (workspace_id, status);
CREATE UNIQUE INDEX uq_credit_top_ups_checkout
    ON maidan_credit_top_ups (checkout_session_id);
CREATE UNIQUE INDEX uq_credit_top_ups_open_amount
    ON maidan_credit_top_ups (chain, amount_micros) WHERE status = 'open';

CREATE TABLE maidan_credit_unmatched (
    chain         TEXT NOT NULL,
    tx_hash       TEXT NOT NULL,
    log_index     INTEGER NOT NULL,
    block_number  INTEGER NOT NULL,
    from_address  TEXT NOT NULL,
    to_address    TEXT NOT NULL,
    amount_micros INTEGER NOT NULL,
    seen_at       TEXT NOT NULL,
    PRIMARY KEY (chain, tx_hash, log_index)
);

CREATE TABLE maidan_credit_cursor (
    chain      TEXT NOT NULL,
    address    TEXT NOT NULL,
    last_block INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (chain, address)
);
