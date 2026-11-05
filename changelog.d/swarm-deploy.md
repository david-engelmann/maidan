### The release deploy can target another hosted instance

- **Added:** `deploy/cloudflare/deploy-release.sh` takes `DEPLOY_CONFIG` and `DEPLOY_ORIGIN`, so the same signed-release deploy that serves maidan.world can deploy a second instance. `wrangler.swarm.jsonc` describes one: `swarm.maidan.world`, the project's own agent swarm, kept apart from the public instance.
