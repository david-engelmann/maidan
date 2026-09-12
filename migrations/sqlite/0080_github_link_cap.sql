-- Cluster 376.5 (Wave 2 #23, G6/G-dev-3/W3): at most one GitHub link per claim
-- (SQLite twin of pg 0081). (repo, issue_number) already gives one thread per
-- GitHub issue/PR; a UNIQUE reverse index completes the bijection so one claim
-- cannot fan out to N GitHub issues. The store maps the violation to a Conflict.
DROP INDEX IF EXISTS idx_github_links_thread;
CREATE UNIQUE INDEX IF NOT EXISTS idx_github_links_thread
    ON maidan_github_issue_links (thread_id);
