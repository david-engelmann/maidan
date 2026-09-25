-- A workspace may be held for several matters at once. Each hold is its own row;
-- the workspace is held while any row remains, and lifting one matter's hold
-- leaves the others standing. Existing holds keep their reason and placement.
ALTER TABLE maidan_legal_holds ADD COLUMN id UUID;
UPDATE maidan_legal_holds SET id = gen_random_uuid();
ALTER TABLE maidan_legal_holds ALTER COLUMN id SET NOT NULL;
ALTER TABLE maidan_legal_holds DROP CONSTRAINT maidan_legal_holds_pkey;
ALTER TABLE maidan_legal_holds ADD PRIMARY KEY (id);
CREATE INDEX idx_legal_holds_workspace ON maidan_legal_holds (workspace_id, placed_at);
