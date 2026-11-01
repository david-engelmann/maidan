### Added
- Console: board cards in review and in progress show vote badges (approve, changes requested, ack) that refresh live on `vote_cast` / `vote_retracted`. One `GET /threads/{tid}/votes` per card (added in #1838). On a server without that route (404), the console falls back to the thread's messages and their per-message votes. Depends on #1838 for the single-call path.
