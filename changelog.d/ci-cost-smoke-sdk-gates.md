### CI cost cron gate and smoke input bounds

- **Added:** `scripts/check-ci-cost.sh` refuses GitHub workflow schedules and Cloudflare `wrangler` crons that fire more than once per day, matching the hosted container cost rule.
- **Fixed:** `otlp-smoke`, `scale-out-smoke`, `mcp-inspector`, and `openapi-lint` validate integer ports in `1..65535` before binding or curling.
- **Fixed:** `release-image-smoke` refuses `latest`, path-like owners, and illegal tag characters before any `docker pull`.
