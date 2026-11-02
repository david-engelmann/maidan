### The console shows and adds a task's evidence records

- **Added:** a task's "Result and evidence" panel in the console lists its evidence records (test runs, CI runs, diffs, outside reviews, screenshots and notes) on the timeline, with a link for each record's URL. Only `http` and `https` links are rendered.
- **Added:** an "Add evidence" form on the same panel records one with `POST /threads/{id}/evidence-records`. It checks the draft the way the server does before sending, and needs a bearer token, as the route does.
