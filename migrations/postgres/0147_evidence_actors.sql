-- Open Work Next 3 (attestation tiers). A hand-off tiers each piece of
-- evidence by who put it there: a worker's evidence is self-reported. A
-- delegate that worked the thread could otherwise link evidence, or set the
-- result, with a token borrowed from a member who never held it, and have it
-- read as attached. Record the delegate beside the member, as approvals and
-- land-gate verdicts already do. NULL when the member acted for itself.
ALTER TABLE maidan_thread_artifacts ADD COLUMN linked_actor_id UUID REFERENCES maidan_members(id);
ALTER TABLE maidan_thread_results ADD COLUMN produced_actor_id UUID REFERENCES maidan_members(id);
