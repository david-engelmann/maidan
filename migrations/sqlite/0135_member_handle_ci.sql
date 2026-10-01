-- A member handle is unique regardless of case. Keep the oldest member of
-- each case-only group (created_at, then id) and rename the rest, so an
-- upgraded database can build the index. The new handle is the old one,
-- '~', and hex(id): member ids are 16-byte blobs, not hyphenated text. If
-- that form is already a handle in the workspace, the id is appended twice.

UPDATE maidan_members
SET handle = CASE
    WHEN EXISTS (
        SELECT 1
        FROM maidan_members AS other
        WHERE other.workspace_id = maidan_members.workspace_id
          AND other.id <> maidan_members.id
          AND lower(other.handle) = lower(
              maidan_members.handle || '~' || hex(maidan_members.id)
          )
    )
    THEN maidan_members.handle || '~' || hex(maidan_members.id)
         || '~' || hex(maidan_members.id)
    ELSE maidan_members.handle || '~' || hex(maidan_members.id)
END
WHERE id IN (
    SELECT id
    FROM (
        SELECT
            id,
            row_number() OVER (
                PARTITION BY workspace_id, lower(handle)
                ORDER BY created_at ASC, id ASC
            ) AS n
        FROM maidan_members
    ) AS ranked
    WHERE n > 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_members_handle_ci
    ON maidan_members (workspace_id, lower(handle));
