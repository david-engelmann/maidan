-- Open Work Next 2 (vote integrity): a member holds at most one verdict on a
-- message. `approve` and `request_changes` exclude each other; `ack` is
-- independent. Where both exist, the later one stands.
DELETE FROM maidan_votes
WHERE kind IN ('approve', 'request_changes')
  AND EXISTS (
    SELECT 1 FROM maidan_votes w
    WHERE w.message_id = maidan_votes.message_id
      AND w.member_id = maidan_votes.member_id
      AND w.kind IN ('approve', 'request_changes')
      AND w.kind <> maidan_votes.kind
      AND (w.created_at > maidan_votes.created_at
           OR (w.created_at = maidan_votes.created_at AND w.kind > maidan_votes.kind))
  );

CREATE UNIQUE INDEX idx_votes_one_verdict ON maidan_votes (message_id, member_id)
    WHERE kind IN ('approve', 'request_changes');
