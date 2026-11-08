### SQLite invites take the write lock first

- **Fixed:** minting a workspace invite on SQLite could fail with `database is locked` when another connection wrote at the same moment: the transaction read the inviter before it wrote, and SQLite refuses to upgrade a stale read to a write without waiting. It now begins with `BEGIN IMMEDIATE`, as accepting an invite already did.
