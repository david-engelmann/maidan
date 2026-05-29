-- Per-model embedding tables (Cluster 47.0).

CREATE TABLE maidan_embedding_models (
    model       TEXT PRIMARY KEY,
    dimension   INTEGER NOT NULL,
    table_name  TEXT NOT NULL UNIQUE,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE maidan_emb_hash_v1 (
    message_id TEXT PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
    embedding  BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO maidan_emb_hash_v1 (message_id, embedding, created_at, updated_at)
SELECT message_id, embedding, created_at, updated_at
FROM maidan_message_embeddings
WHERE model = 'hash-v1'
ON CONFLICT (message_id) DO NOTHING;

INSERT INTO maidan_embedding_models (model, dimension, table_name)
VALUES ('hash-v1', 1024, 'maidan_emb_hash_v1')
ON CONFLICT (model) DO NOTHING;

DROP TABLE maidan_message_embeddings;
