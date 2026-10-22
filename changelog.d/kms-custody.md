### Workspace keys can live in AWS KMS

- **Added:** `MAIDAN_CONTENT_KEY_CUSTODY=aws-kms` wraps each new workspace key with an AWS KMS key of its own, tagged with its workspace and bound to the workspace key by encryption context. The database and the instance KEK together open nothing without KMS. `MAIDAN_KMS_KEY_DELETION_DAYS` (7 to 30, default 7) and `MAIDAN_KMS_ENDPOINT` configure it. `local` (the instance KEK) stays the default.
- **Changed:** erasing or purging a workspace, and the rewrap pass after a workspace key rotation, destroy a KMS-held key by scheduling its deletion before the row goes, so a backup of the row opens nothing afterwards. A KMS that refuses fails the erasure, which then changes nothing. Reads open the workspace keys their rows name before they map the rows.
