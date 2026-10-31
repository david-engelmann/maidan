### Dead egress allowlist entries are marked

- **Added:** `GET /workspaces/{id}/egress-targets` marks an entry that no current rule would bless (a non-ASCII branch base or a ref spelling stored before the rules tightened, or a surface this build cannot deliver to) with a `dead` reason, and logs a warning, so the operator can see it and revoke it. Live entries are unchanged on the wire.
- **Added:** A test checks that a stored non-ASCII branch destination in the egress outbox dead-letters as unroutable without a post.
