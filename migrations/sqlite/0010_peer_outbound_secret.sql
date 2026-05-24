-- Persist encrypted outbound peer bearer for federation pull after restart.

ALTER TABLE maidan_peers
    ADD COLUMN outbound_secret_ciphertext TEXT;
