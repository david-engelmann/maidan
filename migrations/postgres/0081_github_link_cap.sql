-- Cluster 376.5 (Wave 2 #23, G6/G-dev-3/W3): at most one GitHub link per claim.
-- The spawn budget caps internal fan-out (children/depth/tools); this caps the
-- EXTERNAL fan-out of a single claim. `maidan_github_issue_links` is keyed on
-- (repo, issue_number), which already gives one Maidan thread per GitHub
-- issue/PR; making the reverse index UNIQUE completes the bijection, so a runaway
-- agent cannot fan one claim out to N GitHub issues. The store maps the violation
-- to a Conflict (SpawnRejected -> REST 409 / MCP InvalidParams).
--
-- Pre-1.0 note: a deployment whose thread already holds several links must unlink
-- the extras before this migration can apply.
DROP INDEX IF EXISTS idx_github_links_thread;
CREATE UNIQUE INDEX IF NOT EXISTS idx_github_links_thread
    ON maidan_github_issue_links (thread_id);
