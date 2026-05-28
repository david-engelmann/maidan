-- Maidan embeddings schema, v0003, SQLite dialect.
--
-- Per-message embedding vectors (1024-dim float32 blobs). Semantic ranking
-- uses cosine similarity in `maidan-search` (same dimension as Postgres/pgvector).

CREATE TABLE maidan_message_embeddings (
    message_id TEXT PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,
    embedding  BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_message_embeddings_model ON maidan_message_embeddings (model);
