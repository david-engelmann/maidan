### Deliveries cutoff for slash commands, automation retries and A2A push

- **Fixed:** An open sign-up workspace past its deliveries allowance could still make the server send outbound HTTP through slash commands, queued slash-command and FSM-hook retries, and A2A push notifications. A slash command now answers `over_allowance` and queues no retry, a queued retry is deferred with its attempts unchanged, and a push is not sent. Each attempt on these paths, failed or not, now counts one delivery.
