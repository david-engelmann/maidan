-- Who actually made each attestation. See the Postgres twin for the
-- reasoning, including why these carry no foreign key.
ALTER TABLE maidan_thread_reviews ADD COLUMN actor_id TEXT;
ALTER TABLE maidan_thread_land_gate ADD COLUMN recorded_actor_id TEXT;
ALTER TABLE maidan_approval_gates ADD COLUMN requested_actor_id TEXT;
ALTER TABLE maidan_approval_gates ADD COLUMN resolved_actor_id TEXT;
