-- Maidan embeddings schema, v0003, Postgres dialect.
--
-- Per-message embedding vectors. Callers bring their own model (no
-- embedding generation in v0.2.0) and store the vector + model name.
-- HNSW index supports cosine similarity for `<=>` queries.
--
-- Dimension is fixed at 1024 for v0.2.0; future migrations widen or
-- partition by model.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE maidan_message_embeddings (
    message_id UUID PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,
    embedding  vector(1024) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_message_embeddings_hnsw
    ON maidan_message_embeddings
    USING hnsw (embedding vector_cosine_ops);

CREATE INDEX idx_message_embeddings_model ON maidan_message_embeddings (model);
