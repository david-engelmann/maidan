-- Handoffs at non_converging (Next 33, #1464). A Pi task still open past the
-- round limit, or whose diff grew each round, is handed to a session once:
-- its claim ends and its engine becomes `session`. This row makes it once,
-- and the swarm metrics count the task as misrouted.
CREATE TABLE maidan_thread_handoffs (
    thread_id     TEXT PRIMARY KEY REFERENCES maidan_threads(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    repo          TEXT NOT NULL,
    pull_number   INTEGER NOT NULL,
    rounds        INTEGER NOT NULL,
    handed_off_at TEXT NOT NULL
);
