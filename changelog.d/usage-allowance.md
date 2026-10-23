### A workspace past the free allowance is marked and its admins told

- **Added:** `MAIDAN_FREE_ALLOWANCE` (for example `requests=100000,stored_bytes=1073741824`) sets the instance's free allowance per meter (Next 27, part 3). A counter's allowance is per UTC month, and `stored_bytes`' is a ceiling. The value is parsed at boot, and a malformed one refuses to start.
- **Added:** past an allowance, the `usage_meters` loop marks the workspace once per meter and month (migration 0200, both backends). In the same transaction it appends a `usage_over_allowance` event and notifies each workspace admin. Nothing is cut off or billed.
- **Added:** `GET /workspaces/{id}/usage-meters?month=` (`workspace:read`, also served to the board's session) and `GET /operator/usage-meters?month=` (`operator:global`), each meter's month against the allowance. The board's Swarm tab now opens with this month's usage.
