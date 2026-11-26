### A webhook says which token it acts as

- **Added:** `GET /workspaces/{wid}/webhooks` (and the console's twin) names, for each webhook, the token it acts as in an `acts_as` object: the token's id and label (never its secret), and whether it is `live`, `expiring` (with `expires_at`) or `ended`. A webhook is revoked at its next event once its token ends, so a reconcile can see that coming and replace the webhook first. Only a caller who may list the workspace's tokens (`token:admin`) gets `acts_as`; every other caller gets the webhook without it.
