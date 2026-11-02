### A thread's result says whether it was delivered

- **Changed:** `GET /threads/{id}/result` now carries `delivery_state` (`pending`, `delivered`, `failed`, `skipped` or `not_routable`) and the per-target `deliveries` rows, so a stored-but-undelivered result is visible without a second request (#1719). The existing result fields are unchanged.
