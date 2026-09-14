-- Cluster 381: index `result_kind` as a namespaced string facet (e.g.
-- `example.review.result/1`), not a closed enum. Extracted from the opaque result
-- JSON so a waiter product can ship a new kind without a Maidan release.
-- NULL = the payload has no usable `result_kind` (missing / empty / non-string)
-- and the row is stored but not facetable under a kind.
ALTER TABLE maidan_thread_results ADD COLUMN result_kind TEXT;

UPDATE maidan_thread_results
SET result_kind = result->>'result_kind'
WHERE jsonb_typeof(result->'result_kind') = 'string'
  AND length(result->>'result_kind') > 0;

CREATE INDEX IF NOT EXISTS idx_thread_results_result_kind
    ON maidan_thread_results (result_kind)
    WHERE result_kind IS NOT NULL;
