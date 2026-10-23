### Indexer events-behind gauge

- **Added:** `maidan_indexer_events_behind`, the number of message events the indexer has not handled yet (counted up to 100000, 0 when caught up), and the `MaidanIndexerBacklogDeep` alert when it stays past 10000 for 10 minutes.
