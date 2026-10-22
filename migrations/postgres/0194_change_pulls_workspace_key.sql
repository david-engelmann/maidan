-- Tracked pull requests are keyed by workspace as well as repo and number, so
-- two workspaces watching the same pull request each keep their own row
-- instead of the second one being refused or overwriting the first.
UPDATE maidan_change_rounds r
SET workspace_id = p.workspace_id
FROM maidan_change_pulls p
WHERE r.workspace_id IS NULL AND r.repo = p.repo AND r.pull_number = p.pull_number;
ALTER TABLE maidan_change_rounds ALTER COLUMN workspace_id SET NOT NULL;

ALTER TABLE maidan_change_rounds DROP CONSTRAINT maidan_change_rounds_repo_pull_number_fkey;
ALTER TABLE maidan_pull_ci DROP CONSTRAINT maidan_pull_ci_repo_pull_number_fkey;

ALTER TABLE maidan_change_pulls DROP CONSTRAINT maidan_change_pulls_pkey;
ALTER TABLE maidan_change_pulls ADD PRIMARY KEY (workspace_id, repo, pull_number);

ALTER TABLE maidan_change_rounds DROP CONSTRAINT maidan_change_rounds_pkey;
ALTER TABLE maidan_change_rounds ADD PRIMARY KEY (workspace_id, repo, pull_number, commit_sha);
ALTER TABLE maidan_change_rounds ADD CONSTRAINT maidan_change_rounds_pull_fkey
    FOREIGN KEY (workspace_id, repo, pull_number)
    REFERENCES maidan_change_pulls(workspace_id, repo, pull_number) ON DELETE CASCADE;

ALTER TABLE maidan_pull_ci DROP CONSTRAINT maidan_pull_ci_pkey;
ALTER TABLE maidan_pull_ci ADD PRIMARY KEY (workspace_id, repo, pull_number);
ALTER TABLE maidan_pull_ci ADD CONSTRAINT maidan_pull_ci_pull_fkey
    FOREIGN KEY (workspace_id, repo, pull_number)
    REFERENCES maidan_change_pulls(workspace_id, repo, pull_number) ON DELETE CASCADE;
