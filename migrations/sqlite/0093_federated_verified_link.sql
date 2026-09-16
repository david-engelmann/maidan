-- Cluster 397.6: the last origin EventLink *verified* from a peer, which is not
-- the same as the last one ingested.
--
-- The origin chain covers every event in the peer's workspace, but federation
-- only accepts the `federatable()` allowlist. A non-federatable event therefore
-- verifies against the chain and is then refused by policy — and because the
-- link was only recorded on the ingest path, the chain pointer stayed behind it.
-- Every later envelope then failed PrevHashMismatch forever, while the pull
-- worker advanced its cursor past them: a permanently wedged peer plus silent
-- loss of everything after the first ThreadReady.
--
-- One row per peer: what we last checked, whether or not we kept it.
CREATE TABLE maidan_federated_verified_link (
    peer_id             TEXT PRIMARY KEY REFERENCES maidan_peers(id) ON DELETE CASCADE,
    remote_event_id     BIGINT NOT NULL,
    origin_prev_hash    TEXT NOT NULL,
    origin_content_hash TEXT NOT NULL,
    verified_at         TEXT NOT NULL DEFAULT (datetime('now'))
);
