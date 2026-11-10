-- A one-time backfill: a waiting review takes its change's engine and
-- repository; see the Postgres twin (0230).
UPDATE maidan_thread_attributes
SET engine = COALESCE(engine, (
        SELECT pa.engine FROM maidan_threads t
        JOIN maidan_thread_attributes pa ON pa.thread_id = t.parent_thread_id
        WHERE t.id = maidan_thread_attributes.thread_id
    )),
    repo = COALESCE(repo, (
        SELECT pa.repo FROM maidan_threads t
        JOIN maidan_thread_attributes pa ON pa.thread_id = t.parent_thread_id
        WHERE t.id = maidan_thread_attributes.thread_id
    ), (
        SELECT MIN(pf.repo) FROM maidan_threads t
        JOIN maidan_thread_footprints pf ON pf.thread_id = t.parent_thread_id
        WHERE t.id = maidan_thread_attributes.thread_id
    ))
WHERE kind = 'review'
  AND (engine IS NULL OR repo IS NULL)
  AND thread_id IN (
      SELECT t.id FROM maidan_threads t
      WHERE t.parent_thread_id IS NOT NULL
        AND t.state IN ('open', 'in_review')
        AND t.tombstoned_at IS NULL
        AND NOT EXISTS (SELECT 1 FROM maidan_thread_footprints f WHERE f.thread_id = t.id)
        AND (EXISTS (
                SELECT 1 FROM maidan_thread_attributes pa
                WHERE pa.thread_id = t.parent_thread_id AND pa.repo IS NOT NULL
            ) OR EXISTS (
                SELECT 1 FROM maidan_thread_footprints pf WHERE pf.thread_id = t.parent_thread_id
            ))
  );
