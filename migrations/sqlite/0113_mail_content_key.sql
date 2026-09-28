-- Crypto-shredding reaches the mail outbox. A notification email about a
-- message is linked to that message's content key, so withdrawing the message
-- deletes its queued, dead-lettered and sent rows with the key, and purging the
-- workspace deletes them with its keys (ON DELETE CASCADE). A mail about
-- anything else carries no key.
ALTER TABLE maidan_mail_outbox
    ADD COLUMN content_key_id TEXT REFERENCES maidan_content_keys(id) ON DELETE CASCADE;
CREATE INDEX idx_mail_outbox_content_key ON maidan_mail_outbox (content_key_id)
    WHERE content_key_id IS NOT NULL;
