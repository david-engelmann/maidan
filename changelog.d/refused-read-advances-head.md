### A refused checks read still moves the head

- **Fixed:** when GitHub refuses a pull request's checks and its Actions runs and commit statuses too, the change sweep now records the read with the new head and the checks `unreadable` (a new `CiStatus`), instead of `pending` (or nothing at all when GitHub lists no head commit). A push therefore dismisses approvals given to the old head, and no checks_done is said for a result nobody could read (#1748).
