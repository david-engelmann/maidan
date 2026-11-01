### Added
- `GET /threads/{tid}/votes` (`workspace:read`) returns the votes on a thread's messages in one read. For each message holding a vote it gives counts of `approve`, `request_changes` and `ack` plus the vote rows, so a board can badge cards without one call per message. It applies the same access rule as reading the thread.
