-- Open Work Next 3 (attestation tiers). A hand-off tiers each piece of
-- evidence by who put it there: a worker's evidence is self-reported. A
-- delegate that worked the thread could otherwise link evidence, or set the
-- result, with a token borrowed from a member who never held it, and have it
-- read as attached. Record the delegate beside the member, as approvals and
-- land-gate verdicts already do. Every write from here on records who acted:
-- the delegate, or the member itself. NULL marks a row written before this
-- migration, whose actor was never recorded and cannot be backfilled (no
-- event carried it). A hand-off counts such a row as the member's own act
-- when the member never delegated, and as possibly a worker's when it did.
ALTER TABLE maidan_thread_artifacts ADD COLUMN linked_actor_id UUID REFERENCES maidan_members(id);
ALTER TABLE maidan_thread_results ADD COLUMN produced_actor_id UUID REFERENCES maidan_members(id);
