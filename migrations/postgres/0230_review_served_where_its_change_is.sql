-- A one-time backfill the maintainer approved: a second-family review spawned
-- before its change's engine and repository were copied onto it takes them now.
--
-- A seat claims, and wakes on queue depth, filtered by engine and repository.
-- A review child carried neither, so no filtered seat ever counted or claimed
-- it. From this release the spawn copies them (crates/maidan-store/src/
-- {postgres,sqlite}/review_family.rs), and `claim_next`'s repository filter
-- reads the `repo` attribute as well as the footprint, so a review needs no
-- footprint. This fills the reviews still waiting: an open or in-review
-- `kind = review` child with no footprint of its own, whose change names a
-- repository in its attributes or its footprint. Only missing values are
-- filled, the attribute first and the footprint after, so a second run
-- changes nothing; a closed, archived or erased review is left alone.
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
