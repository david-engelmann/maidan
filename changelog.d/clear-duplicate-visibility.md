### Clearing a duplicate link needs sight of the original

- **Fixed:** `DELETE /threads/{id}/fingerprint/duplicate-of` now answers `404` when the caller can't see the original the thread is linked to, or can't see the thread itself (another workspace, a private channel). That is the same answer a missing thread gets, so nothing about another tenant's threads leaks. Each clear writes a `thread.duplicate_cleared` audit event in the thread's workspace, in the same transaction (#1580).
