-- Per-model embedding tables (Cluster 47.0).
-- Replaces single `maidan_message_embeddings` with registry + one table per model.

CREATE TABLE maidan_embedding_models (
    model       TEXT PRIMARY KEY,
    dimension   INTEGER NOT NULL,
    table_name  TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE maidan_emb_hash_v1 (
    message_id UUID PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
    embedding  vector(1024) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_maidan_emb_hash_v1_hnsw
    ON maidan_emb_hash_v1
    USING hnsw (embedding vector_cosine_ops);

INSERT INTO maidan_emb_hash_v1 (message_id, embedding, created_at, updated_at)
SELECT message_id, embedding, created_at, updated_at
FROM maidan_message_embeddings
WHERE model = 'hash-v1'
ON CONFLICT (message_id) DO NOTHING;

INSERT INTO maidan_embedding_models (model, dimension, table_name)
VALUES ('hash-v1', 1024, 'maidan_emb_hash_v1')
ON CONFLICT (model) DO NOTHING;

DO $$
DECLARE
    r RECORD;
    tbl TEXT;
    dim INT := 1024;
BEGIN
    FOR r IN SELECT DISTINCT model FROM maidan_message_embeddings WHERE model <> 'hash-v1'
    LOOP
        tbl := 'maidan_emb_' || regexp_replace(r.model, '[^a-zA-Z0-9]+', '_', 'g');
        EXECUTE format(
            'CREATE TABLE IF NOT EXISTS %I (
                message_id UUID PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
                embedding vector(%s) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )',
            tbl,
            dim
        );
        EXECUTE format(
            'CREATE INDEX IF NOT EXISTS %I ON %I USING hnsw (embedding vector_cosine_ops)',
            'idx_' || tbl || '_hnsw',
            tbl
        );
        EXECUTE format(
            'INSERT INTO %I (message_id, embedding, created_at, updated_at)
             SELECT message_id, embedding, created_at, updated_at
             FROM maidan_message_embeddings
             WHERE model = %L
             ON CONFLICT (message_id) DO NOTHING',
            tbl,
            r.model
        );
        INSERT INTO maidan_embedding_models (model, dimension, table_name)
        VALUES (r.model, dim, tbl)
        ON CONFLICT (model) DO NOTHING;
    END LOOP;
END $$;

DROP TABLE IF EXISTS maidan_message_embeddings;
