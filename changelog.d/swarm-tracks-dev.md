### The swarm runs dev

- **Changed:** `swarm.maidan.world` deploys each `dev` commit whose push CI passed (`deploy swarm`), with the image `deploy dev` built for it, instead of waiting for a tagged release.
