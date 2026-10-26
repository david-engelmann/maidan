### A thread's summary is written once and read by hash

- **Added:** artifact kind `digest` and `GET /threads/{id}/digest`, which returns the newest digest linked to a thread (its sha256, `maidan://artifacts/{sha256}` URI, size, MIME type and linker), or 404 (C10). MCP serves the same pointer as `maidan://digests/{thread_id}`. Readers fetch the bytes once by hash. Migration 0210 widens the artifact-kind check to allow `digest`.
