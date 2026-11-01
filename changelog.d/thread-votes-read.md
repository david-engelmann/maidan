### A thread's votes in one read

- **Added:** `GET /threads/{tid}/votes` (`workspace:read`) returns, for each message in the thread that holds a vote, counts of `approve`, `request_changes` and `ack` plus the vote rows. A board can badge cards without one call per message. It uses the same access rule as reading the thread.
