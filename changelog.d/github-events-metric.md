### Inbound GitHub deliveries are counted

- **Added:** `maidan_github_events_total{event, outcome}` counts each delivery to `POST /integrations/github/events`. Its outcome is `marked`, `unmatched`, `handled`, `ignored` or `refused`, the last for a bad signature. An operator forwarding CI events can check its sent count against what Maidan took in.
