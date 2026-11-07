### Stdio sessions embed messages, and a failed reindex exits non-zero

- **Fixed:** `maidan mcp-stdio` embeds messages on SQLite the same way it does on Postgres. It registers the embedding model, resumes the search projection from the event log, and waits for that projection to finish when stdin closes, so a one-shot pipe leaves the message searchable.
- **Fixed:** `maidan mcp-stdio` takes `--embedding-provider` and `MAIDAN_EMBEDDING_PROVIDER`, the same names as `maidan reindex-embeddings` and the server. An unknown provider fails at startup.
- **Fixed:** `maidan reindex-embeddings` exits non-zero when any message failed to embed, after it prints how many were processed and how many failed.
- **Fixed:** `maidan verify-shredding` applies the same SQLite connection settings as the other commands, and `maidan init` refuses a blank workspace name or admin handle before it opens the database.
