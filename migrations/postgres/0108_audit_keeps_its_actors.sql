-- An audit row keeps naming who acted and for whom, whatever later happens to
-- them. Erasing a workspace deletes its members, and `ON DELETE SET NULL`
-- then blanked the actor and subject of every row they had written, the
-- erase's own row included: the record of who did what went with the data it
-- described. `grant_id` never had a foreign key for the same reason
-- (0105_audit_attribution.sql).
ALTER TABLE maidan_audit
    DROP CONSTRAINT IF EXISTS maidan_audit_actor_id_fkey,
    DROP CONSTRAINT IF EXISTS maidan_audit_subject_id_fkey;
