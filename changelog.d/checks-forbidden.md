### A token that can't read checks says so

- **Fixed:** when GitHub answers a pull request read with a FORBIDDEN error on `statusCheckRollup` (a fine-grained token without Checks: read and Commit statuses: read), the change sweep no longer reads it as "no checks yet". It records the head with the checks `unreadable` (a new `CiStatus`), so a push still dismisses approvals given to the old head, and it logs one warning and posts one operator alert per repo naming the missing permissions (#1623).
