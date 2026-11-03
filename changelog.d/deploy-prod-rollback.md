### Prod deploys snapshot the database and roll back on failure

- **Added:** `deploy prod` snapshots the prod database to a Neon branch (`pre-deploy-<tag>-<time>`, the three newest kept) before it deploys. When the new release fails its `/health` or anonymous-access check, it redeploys the release that was running and fails. The deploy steps live in `deploy/cloudflare/deploy-release.sh`, shared by the deploy and the rollback.
