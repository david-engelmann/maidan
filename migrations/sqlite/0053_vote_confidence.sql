-- Cluster 324 (mirror of postgres 0054): optional confidence weight on a vote
-- for weighted consensus. Nullable; range (0..1) enforced at the API edge.
ALTER TABLE maidan_votes ADD COLUMN confidence REAL;
