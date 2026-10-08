-- Open Work Next 2 (vote integrity): a member holds at most one verdict on a
-- message. `approve` and `request_changes` exclude each other; `ack` is
-- independent. Where both exist, the later one stands.
DELETE FROM maidan_votes v
USING maidan_votes w
WHERE v.message_id = w.message_id
  AND v.member_id = w.member_id
  AND v.kind IN ('approve', 'request_changes')
  AND w.kind IN ('approve', 'request_changes')
  AND v.kind <> w.kind
  AND (v.created_at, v.kind) < (w.created_at, w.kind);

CREATE UNIQUE INDEX idx_votes_one_verdict ON maidan_votes (message_id, member_id)
    WHERE kind IN ('approve', 'request_changes');
