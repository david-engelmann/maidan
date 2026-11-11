### A failure a newer run replaced waits for that run

- **Fixed:** a check that failed in a run that a newer run of the same check on the same head replaced, still running or passed, no longer reports `ci_failed`. That covers a rerun, and a workflow whose first job was cancelled so every later job failed. Newer is the larger GitHub Actions job id. The newer run failing is still a failure, and a failure that cannot be ordered still fails.
