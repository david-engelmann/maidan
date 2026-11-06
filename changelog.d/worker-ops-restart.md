### An operator restart for the hosted container

- **Added:** `POST /__ops/restart` on the Cloudflare Worker, behind the `MAIDAN_OPS_RESTART_TOKEN` bearer, stops the container so the next request starts one with the current settings. A running container otherwise keeps its old environment across deploys that don't change the image.
