### The console shows where results went and where they may go

- **Added:** Tools, Deliveries lists every result delivery in the workspace,
  newest change first, with its target, status (pending, delivered, failed or
  skipped), the reason one did not land, and a link to its task. A status
  filter narrows it, and **Replay** sends a failed or skipped target again and
  updates the row with the server's answer, so a target the workspace still
  does not allow says it stayed skipped.
- **Added:** For a workspace admin, Deliveries also holds the egress allowlist,
  **Where results may go**: each destination with its kind, who allowed it and
  when; a form that checks the selector's shape before sending it and shows
  the server's words for what only the server refuses (a `prod` base); and
  Remove, which asks first. Anyone else reads a sentence, and a token without
  `token:admin` sends no request for it.
- **Added:** `GET /workspaces/{wid}/result-deliveries` (`workspace:read`,
  `?status=` and `limit`), every thread's result deliveries in one list, with
  rows on threads the caller cannot see left out. The console reads it, and
  replays through `POST /ui/api/threads/{tid}/deliveries/{did}/replay`.
- **Added:** `/ui/api/workspaces/{wid}/egress-targets` (GET, POST, and DELETE
  on `{tid}`): the allowlist for a person signed in who holds `token:admin` on
  a live token or may mint the first one. An agent's sign-in, a session made
  from a token, and another workspace's admin are refused; a bearer needs
  `token:admin`, as on the bearer route.
- **Added:** An allowlist entry records who allowed it (`created_by`,
  migration 0241). A re-bless keeps the first blesser.
