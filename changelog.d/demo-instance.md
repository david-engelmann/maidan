### A resetting demo instance for try.maidan.world, built up to the deploy

- **Added:** `DEMO_TOKEN_TTL_SECS` makes `scripts/demo-board.sh` mint every token with an `expires_at`, so a public demo's tokens expire on their own.
- **Added:** `deploy/cloudflare/wrangler.demo.jsonc` and `src/demo.js`, a Worker for `try.maidan.world` that is not deployed by any workflow. Its container (`deploy/cloudflare/demo/`) seeds a fresh SQLite workspace from `scripts/demo-board.sh` on boot, with bootstrap on only for a loopback seed listener. A cron destroys the container every six hours, and the next request reseeds. The container has no internet access and holds no production secret, rate limits and the free-allowance meters are on, sign-up is off, and bodies over 64 KiB are refused at the edge.
- **Changed:** `docs/Listing.md` documents the demo instance and how to deploy it, and `listings/docker/server.yaml` gains an `about` block with the title, description and 400×400 icon.
