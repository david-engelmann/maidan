-- Next 17, the inline card: where the decided-via client's name came from.
-- See the Postgres twin.
ALTER TABLE maidan_approval_gates
    ADD COLUMN decided_via_source TEXT
        CHECK (decided_via_source IN ('credential', 'self_reported', 'none'));
ALTER TABLE maidan_approval_gates ADD COLUMN decided_via_client_id TEXT;
UPDATE maidan_approval_gates
   SET decided_via_source = CASE WHEN decided_via_client IS NULL THEN 'none' ELSE 'self_reported' END
 WHERE model_asked = 1;

ALTER TABLE maidan_approval_confirmations
    ADD COLUMN client_source TEXT NOT NULL DEFAULT 'none'
        CHECK (client_source IN ('credential', 'self_reported', 'none'));
ALTER TABLE maidan_approval_confirmations ADD COLUMN client_id TEXT;
UPDATE maidan_approval_confirmations
   SET client_source = 'self_reported'
 WHERE client_name IS NOT NULL;
