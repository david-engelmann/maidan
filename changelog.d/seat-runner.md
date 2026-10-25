### Seat runner for session claimants

- **Added:** `examples/seat-runner/` keeps N session seats claiming, one task per harness run. Before each run it checks the host's load and free disk, polls the queue depth for its engine and repository, and runs the repository's preflight. All of that happens outside the harness sandbox. It then claims, runs the whole harness under `capped`, and always releases the claim, on SIGTERM too. Each harness has a model allowlist, and every run writes one JSON log line.
