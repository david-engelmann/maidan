### Message search always filters by a reader

- **Security:** `maidan_search::SearchFilters.reader_id` is now a required member id, and lexical, semantic and hybrid search on both backends always keep only the threads that member may read. There is no longer a `reader_id: None` search that returns every thread's messages, DMs and private channels included. `SearchFilters` loses `Default` and gains `for_reader(reader)`. REST and MCP search already passed the caller, so what they return is unchanged.
