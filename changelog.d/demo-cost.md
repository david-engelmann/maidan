### The demo sleeps after 15 idle minutes, and its reset never wakes it

- **Changed:** try.maidan.world's container sleeps after 15 idle minutes, not 30. The 6-hourly reset destroys a running container and leaves a sleeping one asleep. `deploy/cloudflare/test/cost.test.js` holds one basic instance, the 15-minute sleep and the destroy-only cron.
