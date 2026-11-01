### The console has an Activity screen

- **Added:** an Activity screen (`#/activity`) streams workspace events newest first. It reads `GET /workspaces/{id}/events` once, then stays live from the console's existing connection, with Tasks / Messages / Votes / Members filters and a pause that holds new rows.
