-- Next 17, the inline card: where the decided-via client's name came from.
--
-- `credential` means the name and id come from what the token was issued to
-- (an installed app today, a registered OAuth client once Next 23 lands): the
-- server vouches for it. `self_reported` means the request's `clientInfo`
-- named it, which proves nothing. `none` means the model's request named no
-- client at all. `decided_via_client_id` is the credential client's id and is
-- empty for the other two. A confirmation carries the same pair from the
-- request that issued it to the decision it records.
--
-- Rows a model decided before this migration got their name from `clientInfo`
-- only, so they read back as `self_reported` (or `none` with no name).
ALTER TABLE maidan_approval_gates
    ADD COLUMN decided_via_source TEXT
        CHECK (decided_via_source IN ('credential', 'self_reported', 'none')),
    ADD COLUMN decided_via_client_id TEXT;
UPDATE maidan_approval_gates
   SET decided_via_source = CASE WHEN decided_via_client IS NULL THEN 'none' ELSE 'self_reported' END
 WHERE model_asked;

ALTER TABLE maidan_approval_confirmations
    ADD COLUMN client_source TEXT NOT NULL DEFAULT 'none'
        CHECK (client_source IN ('credential', 'self_reported', 'none')),
    ADD COLUMN client_id TEXT;
UPDATE maidan_approval_confirmations
   SET client_source = 'self_reported'
 WHERE client_name IS NOT NULL;
