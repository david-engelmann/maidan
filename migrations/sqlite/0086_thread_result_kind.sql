-- Cluster 381 (mirror of postgres 0087): index `result_kind` as a namespaced
-- string facet. SQLite stores the payload as TEXT JSON — extract with json1.
ALTER TABLE maidan_thread_results ADD COLUMN result_kind TEXT;

UPDATE maidan_thread_results
SET result_kind = json_extract(result, '$.result_kind')
WHERE typeof(json_extract(result, '$.result_kind')) = 'text'
  AND json_extract(result, '$.result_kind') != '';

CREATE INDEX IF NOT EXISTS idx_thread_results_result_kind
    ON maidan_thread_results (result_kind)
    WHERE result_kind IS NOT NULL;
