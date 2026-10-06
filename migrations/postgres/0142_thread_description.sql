-- Open Work Next 3 (board write hygiene): threads gain an optional description,
-- set at creation via `CreateThread.description`.
ALTER TABLE maidan_threads ADD COLUMN description TEXT;
