### Store listings always filter by a reader

- **Security:** every store listing that filters by who may read a thread (workspace thread pages and attribute search, pending approval gates, A2A tasks, queue depth and occupancy, live claims, deferred threads, tombstones, the event census, message backlinks, and the workspace context pack) now takes the reader as a required member id. There is no longer a "no reader" call that returns every row, and the shared SQL filter shows no thread row to an unbound reader.
