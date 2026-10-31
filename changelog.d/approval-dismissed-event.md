### A push that dismisses approvals says so

- **Fixed:** A CI read that finds a tracked change's head moved now appends and publishes one `approval_dismissed` event per approval it dismisses, naming the reviewer, the approved head and the new head, so waiters see the gate reopen (#1572).
