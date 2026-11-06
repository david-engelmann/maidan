### Wrangler host contract and fail-closed federation/OAuth smokes

- **Added:** `scripts/check-wrangler-hosts.sh` refuses `workers_dev` and non-`maidan.world` custom domains, and requires `MAIDAN_PUBLIC_ORIGIN` to match a route host over https.
- **Fixed:** `check-release-records.sh` no longer depends on `rg` or swallows scan failures when reading README and Capabilities pins.
- **Fixed:** Federation, MCP OAuth, OAuth-dev-provider, and Keycloak OIDC smokes reject non-http(s) or credentialed base URLs before any network call.
- **Fixed:** `helm-template-smoke.sh` refuses to skip when `helm` is missing under CI/`GITHUB_ACTIONS`.
