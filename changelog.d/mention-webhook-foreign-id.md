### The mention webhook no longer reveals another workspace's subscription ids

- **Fixed:** `PUT /workspaces/{wid}/mention-webhook` now answers a missing subscription id with the same 400 it gives another workspace's id or a revoked one, so a caller can no longer test whether an id exists in some other workspace (#1672).
