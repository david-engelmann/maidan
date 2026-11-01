### The launch surfaces: docs.maidan.world and two ways to run it

- **Added:** a `maidan-docs` Worker config (`deploy/cloudflare/wrangler.docs.jsonc`) serves the docs at `docs.maidan.world` as static assets, and a `docs.maidan.world` job in the docs workflow builds and deploys it from `main` once the `DOCS_DOMAIN` variable is set after the DNS step. The runbook is in `docs/Production.md`, "The docs site".
- **Changed:** the README and the landing page lead with the hosted room at maidan.world and the self-hosted path side by side.
