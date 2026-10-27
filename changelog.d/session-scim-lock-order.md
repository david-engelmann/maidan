### Fixed
- On Postgres, a sign-in that overlaps a SCIM delete or deactivation no longer deadlocks against it. The session write now locks the SCIM row before the member row, the same order the SCIM writes use, so the sign-in waits and is refused. The SCIM row is locked whether or not it is active, so a deactivation in flight is also waited for (#1670).
