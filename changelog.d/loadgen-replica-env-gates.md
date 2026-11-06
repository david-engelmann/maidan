### Fixed

- **Security:** `scripts/loadgen.sh` now fail-closes on external URL/bearer/IDS and concurrency bounds before invoking cargo, and `scripts/replica-harness.sh` refuses privileged or colliding host ports.
