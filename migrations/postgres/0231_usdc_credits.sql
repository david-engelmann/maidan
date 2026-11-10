-- Prepaid credits paid in USDC on Base (docs/Credits.md).
--
-- The ledger: deposits and operator grants add, overage charges take, and a
-- workspace's balance is the sum of its rows, in USD micros (one USDC base
-- unit). No foreign key to the workspace: a money record outlives an erase,
-- and a forced restore, which drops and rebuilds the workspace row, must not
-- take the balance with it.
CREATE TABLE maidan_credit_ledger (
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('deposit', 'charge', 'grant')),
    source        TEXT CHECK (source IN ('usdc_base', 'stripe', 'grant')),
    external_ref  TEXT,
    usd_micros    BIGINT NOT NULL,
    unpaid_micros BIGINT NOT NULL DEFAULT 0,
    meter         TEXT,
    month         TEXT,
    from_units    BIGINT,
    through_units BIGINT,
    chain         TEXT,
    tx_hash       TEXT,
    log_index     BIGINT,
    top_up_id     UUID,
    note          TEXT,
    created_at    TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_credit_ledger_workspace ON maidan_credit_ledger (workspace_id, created_at);
-- Money comes in by more than one rail (USDC on Base now, card payments
-- through Stripe next), each with its own reference: the transaction hash and
-- log index of a transfer, the Checkout Session id of a card payment. A
-- replayed log, a webhook delivered twice, or a second replica reading the
-- same block never credits twice.
CREATE UNIQUE INDEX uq_credit_ledger_source_ref
    ON maidan_credit_ledger (source, external_ref);
-- Each charge covers the units (from_units, through_units] past the
-- allowance and starts where the last one ended, so two replicas charging
-- from the same point collide here and one of them charges nothing.
CREATE UNIQUE INDEX uq_credit_ledger_charge
    ON maidan_credit_ledger (workspace_id, meter, month, from_units) WHERE kind = 'charge';

-- Top-up intents: the exact amount a workspace was asked to send. An open
-- intent's amount is unique on its chain across the instance, which is how a
-- plain transfer is matched to its workspace.
CREATE TABLE maidan_credit_top_ups (
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL,
    method        TEXT NOT NULL DEFAULT 'usdc' CHECK (method IN ('usdc', 'card')),
    chain         TEXT,
    address       TEXT,
    checkout_session_id TEXT,
    checkout_url  TEXT,
    usd           BIGINT NOT NULL,
    amount_micros BIGINT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('open', 'paid', 'expired')),
    created_at    TIMESTAMPTZ NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    paid_at       TIMESTAMPTZ
);
CREATE INDEX idx_credit_top_ups_workspace ON maidan_credit_top_ups (workspace_id, status);
CREATE UNIQUE INDEX uq_credit_top_ups_checkout
    ON maidan_credit_top_ups (checkout_session_id);
CREATE UNIQUE INDEX uq_credit_top_ups_open_amount
    ON maidan_credit_top_ups (chain, amount_micros) WHERE status = 'open';

-- Transfers to the receiving address that matched no intent, kept for the
-- operator: never dropped.
CREATE TABLE maidan_credit_unmatched (
    chain         TEXT NOT NULL,
    tx_hash       TEXT NOT NULL,
    log_index     BIGINT NOT NULL,
    block_number  BIGINT NOT NULL,
    from_address  TEXT NOT NULL,
    to_address    TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    seen_at       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (chain, tx_hash, log_index)
);

-- The deposit watcher's place: the last block scanned for each chain and
-- receiving address.
CREATE TABLE maidan_credit_cursor (
    chain      TEXT NOT NULL,
    address    TEXT NOT NULL,
    last_block BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (chain, address)
);
