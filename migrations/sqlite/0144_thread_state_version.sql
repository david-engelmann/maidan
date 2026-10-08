-- Open Work Next 3 (evidence-bound approvals), part 1. A thread's version
-- counts the writes to what a reviewer reads of it: its messages, its result,
-- its title and description, and the artifacts linked to it. The database
-- bumps it, so no write path can change that content without moving it, and an
-- approval can name the version it was shown.
CREATE TABLE maidan_thread_versions (
    thread_id TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    version INTEGER NOT NULL
);

-- An artifact attached to a thread as evidence, by content hash.
CREATE TABLE maidan_thread_artifacts (
    thread_id TEXT NOT NULL REFERENCES maidan_threads(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    linked_by TEXT NOT NULL REFERENCES maidan_members(id),
    linked_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, sha256)
);

-- A row is written only while the thread exists: a thread being deleted takes
-- its version with it.

CREATE TRIGGER maidan_messages_version_insert
AFTER INSERT ON maidan_messages
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_messages_version_delete
AFTER DELETE ON maidan_messages
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT OLD.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = OLD.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_messages_version_change
AFTER UPDATE ON maidan_messages
WHEN OLD.body IS NOT NEW.body OR OLD.metadata IS NOT NEW.metadata OR OLD.content IS NOT NEW.content OR OLD.tombstoned_at IS NOT NEW.tombstoned_at
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_thread_results_version_insert
AFTER INSERT ON maidan_thread_results
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_thread_results_version_update
AFTER UPDATE ON maidan_thread_results
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_thread_results_version_delete
AFTER DELETE ON maidan_thread_results
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT OLD.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = OLD.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_thread_artifacts_version_insert
AFTER INSERT ON maidan_thread_artifacts
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_thread_artifacts_version_delete
AFTER DELETE ON maidan_thread_artifacts
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT OLD.thread_id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = OLD.thread_id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;

CREATE TRIGGER maidan_threads_version
AFTER UPDATE ON maidan_threads
WHEN OLD.title IS NOT NEW.title OR OLD.description IS NOT NEW.description
BEGIN
    INSERT INTO maidan_thread_versions (thread_id, version)
    SELECT NEW.id, 1 WHERE EXISTS (SELECT 1 FROM maidan_threads WHERE id = NEW.id)
    ON CONFLICT (thread_id) DO UPDATE SET version = maidan_thread_versions.version + 1;
END;
