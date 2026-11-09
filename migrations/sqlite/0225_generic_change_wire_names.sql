-- The change producer's wire names become generic; see the Postgres twin
-- (0226). Old rows are left unrouted, not renamed.
--
-- SQLite cannot alter a CHECK in place. Rather than rebuild two tables (the
-- workspace table is the parent of most others), each engine column is
-- swapped for a new one carrying the new CHECK: add it, copy, drop the old
-- one (its index first, since an indexed column cannot be dropped) and
-- rename. The rename rewrites the CHECK to name the column.
UPDATE maidan_thread_attributes SET engine = NULL WHERE engine = 'pi';
DROP INDEX idx_thread_attributes_engine;
ALTER TABLE maidan_thread_attributes
    ADD COLUMN engine_next TEXT CHECK (engine_next IN ('unattended', 'session', 'human'));
UPDATE maidan_thread_attributes SET engine_next = engine;
ALTER TABLE maidan_thread_attributes DROP COLUMN engine;
ALTER TABLE maidan_thread_attributes RENAME COLUMN engine_next TO engine;
CREATE INDEX idx_thread_attributes_engine ON maidan_thread_attributes (engine);

UPDATE maidan_workspaces SET default_engine = NULL WHERE default_engine = 'pi';
ALTER TABLE maidan_workspaces
    ADD COLUMN default_engine_next TEXT
    CHECK (default_engine_next IN ('unattended', 'session', 'human'));
UPDATE maidan_workspaces SET default_engine_next = default_engine;
ALTER TABLE maidan_workspaces DROP COLUMN default_engine;
ALTER TABLE maidan_workspaces RENAME COLUMN default_engine_next TO default_engine;

ALTER TABLE maidan_result_deliveries
    RENAME COLUMN template_warnings TO delivery_warnings;
