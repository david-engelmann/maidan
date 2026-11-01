### Added
- Console: conversations show the votes on each message (approve, changes requested, ack) with counts and voter names, read from the existing `GET /messages/{id}/votes` and refreshed live on `vote_cast` / `vote_retracted` events.
