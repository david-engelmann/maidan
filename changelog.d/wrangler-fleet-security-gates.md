### Changed

- **Security:** Cloudflare wrangler configs now refuse `workers_dev`, multi-instance containers, secret-shaped vars, non-`maidan.world` routes, and any docs-Worker code/cron wake — locked by `scripts/check-wrangler-fleet-gates.sh`.
