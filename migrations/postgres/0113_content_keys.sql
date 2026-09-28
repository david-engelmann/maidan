-- Crypto-shredding. The words of a message event are sealed under a key that
-- belongs to that one message, stored here wrapped by the server's
-- key-encryption key (KEK). Withdrawing the message shreds the key: the wrapped
-- bytes are erased and `shredded_at` records when. The event keeps its
-- ciphertext, so the hash chain still verifies and nobody can read the words.
--
-- `id` is the message id for a message written here, and a name-based id under
-- the peer's namespace for one that arrived by federation, so a peer can only
-- shred keys for messages it sent. `kek_id` is the fingerprint of the KEK that
-- wrapped the key; rotation rewraps rows whose `kek_id` is not the primary.
CREATE TABLE maidan_content_keys (
    id           UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    kek_id       TEXT,
    wrapped_key  BYTEA,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    shredded_at  TIMESTAMPTZ,
    CHECK ((shredded_at IS NULL) = (wrapped_key IS NOT NULL AND kek_id IS NOT NULL))
);
CREATE INDEX idx_content_keys_workspace ON maidan_content_keys (workspace_id);
CREATE INDEX idx_content_keys_kek ON maidan_content_keys (kek_id)
    WHERE shredded_at IS NULL;

-- The key that seals an event's words. Every read joins it to hand the event
-- its key while the words are live.
ALTER TABLE maidan_events
    ADD COLUMN content_key_id UUID REFERENCES maidan_content_keys(id);
CREATE INDEX idx_events_content_key ON maidan_events (content_key_id)
    WHERE content_key_id IS NOT NULL;
