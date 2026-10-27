### Lint CI job has headroom for a cold run

- **Fixed:** The `lint (fmt + clippy + deny)` CI job timeout is raised from 15 to 25 minutes, so a cold-cache PR run on a slower runner is no longer cancelled at 15:00.
