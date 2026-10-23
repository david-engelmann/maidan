### A thread's transition history is readable over the API

- **Added:** `GET /threads/{id}/transitions` lists a thread's state transitions oldest first, each with its actor and time, so a surface can show who moved what when and an approval card can name who requested the review. The rows were recorded from the start but had no read route. Takes an optional `limit` (default 100, at most 500) and requires `workspace:read` plus access to the thread.
