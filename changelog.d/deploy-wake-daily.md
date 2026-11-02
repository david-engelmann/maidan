### Hosted instances wake once a day, not hourly

- **Changed:** The cron trigger on maidan.world and dev.maidan.world now wakes the container daily (09:17 UTC) instead of hourly. Each wake kept the container and Neon's compute running for the 15 minutes of `sleepAfter`, so an idle instance was billed about a quarter of every day. On an idle instance, scheduled work now waits up to a day; any request runs it.
