-- The task attribute key sets a workspace keeps unique among its open
-- threads, as a JSON array of arrays of attribute keys
-- (`[["linear_issue", "repo"]]`: one open thread per Linear ticket in each
-- repository). Empty, the default, keeps nothing unique. The rule is
-- enforced in the thread writes, not by an index: whether values are held
-- depends on this setting and on the other thread's state.
ALTER TABLE maidan_workspaces
    ADD COLUMN unique_thread_attributes TEXT NOT NULL DEFAULT '[]';

-- The repository a task changes (`owner/name`), so a ticket split across
-- repositories can have one owner in each.
ALTER TABLE maidan_thread_attributes ADD COLUMN repo TEXT;
