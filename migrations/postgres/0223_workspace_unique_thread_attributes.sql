-- The task attributes a workspace keeps unique among its open threads, as a
-- JSON array of attribute keys (`["linear_issue"]`: one open thread per
-- Linear ticket). Empty, the default, keeps nothing unique. The rule is
-- enforced in the thread writes, not by an index: whether a value is unique
-- depends on this setting and on the other thread's state.
ALTER TABLE maidan_workspaces
    ADD COLUMN unique_thread_attributes TEXT NOT NULL DEFAULT '[]';
