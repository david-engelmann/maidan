### Activity opens on the newest events, display names keep real joiners, and aging reads one clock

- **Added:** `GET /workspaces/{wid}/events` (and its `/ui/api` twin) takes `order=desc`, which returns the newest events first, and `before_id` to page back from there. `after_id` and `consumer_id` still page forward and are refused with `order=desc` (#1967).
- **Fixed:** the console's Activity screen reads one newest-first page instead of walking forward from the first event, which on a workspace with more than 5,000 events showed events from the middle of the log. The Tools event list and a board's review refusals read newest first too (#1967).
- **Fixed:** the console's radius test reads every value of a `border-radius` shorthand, and the stray 12px and 18px corners in the stylesheet now come from the radius scale (#1967).
- **Fixed:** a display name may carry ZWNJ or ZWJ between two letters of the same joining script (Persian, Devanagari and other Indic names), ZWJ inside an emoji sequence, and VS16 right after an emoji. Leading, trailing, doubled or cross-script joiners, joiners between Latin letters, and a lone VS16 are still refused (#1576).
- **Fixed:** `claim_next`'s priority aging measures a thread's wait on one clock: Postgres reads the database's `NOW()` on both sides, and SQLite the claim's own `now` (#1586).
