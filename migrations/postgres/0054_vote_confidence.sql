-- Cluster 324 (fidelity arc): an optional confidence weight on a vote, so a
-- consumer can compute weighted consensus rather than a flat tally. Nullable —
-- a vote without a stated confidence is the common case. Range is a convention
-- (0..1), enforced at the API edge, not the column.
ALTER TABLE maidan_votes ADD COLUMN confidence DOUBLE PRECISION;
