### Local runs are capped on macOS too

- **Added:** `scripts/capped` runs a command in its own process group and kills the group past a wall-clock or resident-memory cap (exit 124 or 137). macOS has no `timeout` and ignores `ulimit -v`, and a test leaked to 171 GB on the shared machine. The claimant runbook, `docs/Conventions.md` and CLAUDE.md name it, and `scripts/check-capped.sh` runs in the `lint` job.
