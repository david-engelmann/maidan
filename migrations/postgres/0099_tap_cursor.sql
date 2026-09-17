-- Cluster 402.2: a resume cursor for the search tap.
--
-- `backfill_search` walked from id 0 unconditionally — on process start, on
-- every resubscribe, and on every `Lagged`. On Postgres the handler is
-- `BatchingEmbeddingHandler`, so that re-embedded the entire history each time.
-- It also livelocked: the bus is not drained *during* a backfill, so a busy
-- instance overflows the broadcast while walking, gets `Lagged` on the first
-- poll, and walks from 0 again — never converging.
--
-- One row per surface. The tap walks the *global* log in id order, so every
-- workspace advances together and a single high-water is the honest shape. The
-- per-workspace state that genuinely differs — which chains are faulted
-- (Cluster 402.1) — is not a cursor and is deliberately not stored here: a
-- faulted workspace must re-verify from scratch after a rebuild, not resume.
--
-- Deliberately NOT `maidan_delivery_cursor`: retention's floor is
-- `min_delivery_cursor`, so registering the tap there would let a stuck tap
-- block log pruning forever. Keeping it separate means a stuck tap hits
-- `CursorTooOld` and rebuilds — the designed behaviour — instead of growing the
-- log without bound.
CREATE TABLE IF NOT EXISTS maidan_tap_cursor (
    surface        TEXT PRIMARY KEY,
    last_event_id  BIGINT NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
