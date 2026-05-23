-- Maidan search schema, v0002, Postgres dialect.
--
-- Adds a generated tsvector column `search_vec` to `maidan_messages`
-- maintained automatically by Postgres on INSERT / UPDATE. A GIN index
-- backs lexical search via `plainto_tsquery` and `to_tsquery`.
--
-- The vector incorporates the message body weighted A and the topic
-- (if present in metadata) weighted B, matching `ts_headline` snippet
-- behavior. Tombstoned messages still have a vector — search callers
-- filter `tombstoned_at IS NULL` at the query layer.

ALTER TABLE maidan_messages
    ADD COLUMN search_vec tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(body, '')), 'A')
        || setweight(
            to_tsvector(
                'english',
                coalesce(metadata->>'topic', '')
            ),
            'B'
        )
    ) STORED;

CREATE INDEX idx_messages_search_vec ON maidan_messages USING GIN (search_vec);
