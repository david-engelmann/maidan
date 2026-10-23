### Stalled-claim alert pages once per instance

- **Changed:** `MaidanClaimsRenewingWithoutProgress` takes `max without (instance, pod)`, so replicas exporting the same instance-wide count page once. A failed stalled-thread count logs a warning instead of silently leaving the gauge stale, and the count runs on the read pool.
