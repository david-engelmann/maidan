### A resumed release still waits for its publish and deploy

- **Fixed:** a `cut release` rerun that skipped the record job also skipped "wait for the release and maidan.world", because the default `success()` check looks through `promote` to the skipped record; it now runs whenever `promote` succeeded.
