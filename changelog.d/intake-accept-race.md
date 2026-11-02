### Intake: concurrent accepts leave no orphan thread

- **Fixed:** Accepting an intake item into a channel now creates its thread inside the triage transaction. When two accepts race, the loser gets `409` and its thread rolls back with it, so no orphan thread is left in the channel.
