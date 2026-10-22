### Workspace keys can be held by AWS KMS

- **Added:** `MAIDAN_WORKSPACE_KEY_CUSTODY=aws-kms` with `MAIDAN_AWS_KMS_KEY_ID` wraps each workspace's key with an AWS KMS key, bound to the workspace key by encryption context, instead of the instance KEK. Custody sits behind one async interface (local, AWS KMS, and a fake KMS for tests); every read opens the workspace keys its rows need through the custody that wrapped them before it maps them, and the rewrap pass moves keys from the old custody to the configured one. Unknown custody values, or a KMS key without `aws-kms`, refuse boot.
