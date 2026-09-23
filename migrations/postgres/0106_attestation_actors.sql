-- Who actually made each attestation, beside the member it was made for.
--
-- A delegated token *is* the member it acts for, so `reviewer_id`,
-- `recorded_by` and `requested_by`/`resolved_by` record the subject. A
-- separation-of-duties check that reads only those can be laundered: a
-- delegate that did the work approves it by borrowing a reviewer's token.
-- These record the actor, so the checks can refuse an actor approving its
-- own work. NULL means the subject acted for itself (and every row written
-- before this migration).
--
-- No foreign keys, as with `maidan_audit.grant_id`: the record must keep
-- naming who acted whatever later happens to that member, and a SET NULL
-- would quietly weaken the check back to the subject alone.
ALTER TABLE maidan_thread_reviews ADD COLUMN actor_id UUID;
ALTER TABLE maidan_thread_land_gate ADD COLUMN recorded_actor_id UUID;
ALTER TABLE maidan_approval_gates
    ADD COLUMN requested_actor_id UUID,
    ADD COLUMN resolved_actor_id UUID;
