-- The change producer's wire names become generic (Next 44): the task engine
-- `pi` is now `unattended`, and the delivery row's `template_warnings` is
-- `delivery_warnings`, since it carries the result's check codes as well as
-- the template's.
--
-- No alias and no backfill (F-54): a thread or workspace still routed to the
-- old value is left unrouted, as if nobody had routed it, and the old value
-- is refused from here on like any unknown engine. Nulled rather than
-- deleted, so a thread keeps its other attributes.
UPDATE maidan_thread_attributes SET engine = NULL WHERE engine = 'pi';
ALTER TABLE maidan_thread_attributes
    DROP CONSTRAINT maidan_thread_attributes_engine_check;
ALTER TABLE maidan_thread_attributes
    ADD CONSTRAINT maidan_thread_attributes_engine_check
    CHECK (engine IN ('unattended', 'session', 'human'));

UPDATE maidan_workspaces SET default_engine = NULL WHERE default_engine = 'pi';
ALTER TABLE maidan_workspaces
    DROP CONSTRAINT maidan_workspaces_default_engine_check;
ALTER TABLE maidan_workspaces
    ADD CONSTRAINT maidan_workspaces_default_engine_check
    CHECK (default_engine IN ('unattended', 'session', 'human'));

ALTER TABLE maidan_result_deliveries
    RENAME COLUMN template_warnings TO delivery_warnings;
