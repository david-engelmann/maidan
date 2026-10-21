-- A workspace's default engine: the engine a thread created there with
-- none named is routed to, so work Maidan itself creates (schedules, inbound
-- A2A, messages turned into threads, plan children) reaches an
-- engine-filtered claimer. NULL keeps such threads unrouted. The values are
-- the ones `maidan_thread_attributes.engine` takes.
ALTER TABLE maidan_workspaces
    ADD COLUMN default_engine TEXT CHECK (default_engine IN ('pi', 'session', 'human'));
