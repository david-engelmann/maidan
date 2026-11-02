### No delivery email is an answer, and the thread view fits the window

- **Changed:** `GET /members/{id}/email` answers `200` with `null` when the member has set no address, instead of `404`, matching the MCP `get_member_email` tool. A `404` now means only an unknown member. The console's Preferences screen no longer logs an error for a member without an email.
- **Fixed:** At desktop widths the console's Threads screen fits the window instead of running 18px past it at 1280×900.
