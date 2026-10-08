-- Open Work Next 3 (evidence-bound approvals), part 1. A thread's version
-- counts the writes to what a reviewer reads of it: its messages, its result,
-- its title and description, and the artifacts linked to it. The database
-- bumps it, so no write path can change that content without moving it, and an
-- approval can name the version it was shown.
CREATE TABLE maidan_thread_versions (
    thread_id UUID PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    version BIGINT NOT NULL
);

-- An artifact attached to a thread as evidence, by content hash.
CREATE TABLE maidan_thread_artifacts (
    thread_id UUID NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    linked_by UUID NOT NULL REFERENCES maidan_members(id),
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, sha256)
);

-- A thread being deleted takes its content with it; its version goes too, so
-- a row is written only while the thread exists.
CREATE FUNCTION maidan_bump_thread_version(tid UUID) RETURNS void AS $$
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT tid, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = tid)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
$$ LANGUAGE sql;

CREATE FUNCTION maidan_thread_version_by_thread_id() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM maidan_bump_thread_version(OLD.thread_id);
    ELSE
        PERFORM maidan_bump_thread_version(NEW.thread_id);
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION maidan_thread_version_by_id() RETURNS trigger AS $$
BEGIN
    PERFORM maidan_bump_thread_version(NEW.id);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER maidan_messages_version_write AFTER INSERT OR DELETE ON maidan_messages
    FOR EACH ROW EXECUTE FUNCTION maidan_thread_version_by_thread_id();
CREATE TRIGGER maidan_messages_version_change AFTER UPDATE ON maidan_messages
    FOR EACH ROW WHEN (
        OLD.body IS DISTINCT FROM NEW.body
        OR OLD.metadata IS DISTINCT FROM NEW.metadata
        OR OLD.content IS DISTINCT FROM NEW.content
        OR OLD.tombstoned_at IS DISTINCT FROM NEW.tombstoned_at
    )
    EXECUTE FUNCTION maidan_thread_version_by_thread_id();
CREATE TRIGGER maidan_thread_results_version AFTER INSERT OR UPDATE OR DELETE ON maidan_thread_results
    FOR EACH ROW EXECUTE FUNCTION maidan_thread_version_by_thread_id();
CREATE TRIGGER maidan_thread_artifacts_version AFTER INSERT OR DELETE ON maidan_thread_artifacts
    FOR EACH ROW EXECUTE FUNCTION maidan_thread_version_by_thread_id();
CREATE TRIGGER maidan_threads_version AFTER UPDATE ON maidan_threads
    FOR EACH ROW WHEN (
        OLD.title IS DISTINCT FROM NEW.title
        OR OLD.description IS DISTINCT FROM NEW.description
    )
    EXECUTE FUNCTION maidan_thread_version_by_id();
