### Search keeps a broken workspace broken, and embeddings stay bounded

- **Fixed:** A workspace whose search chain broke is not projected by later live events, and a lagged bus no longer forgets that fault or saves a cursor past the gap. Dropping the indexer handle aborts its task.
- **Fixed:** Lexical search, semantic search, and reindex skip messages on a tombstoned thread. A non-positive limit returns no hits instead of an unbounded or rejected query.
- **Fixed:** The Postgres repair lock is not returned to the pool if the sweep is cancelled. An OpenAI-compatible embedder rejects a blank key, a zero timeout, a bad endpoint, and a key sent in cleartext off this machine. It does not follow redirects, and a bad batch of vectors is retried instead of attached to the wrong messages.
