### An Alloy config for a local OTLP workbench

- **Added:** `deploy/observability/alloy/config.alloy` receives the server's OTLP traces and metrics, scrapes `/metrics` (including the usage counters), and deletes prompt, diff, rendered and body span attributes before export. The OTLP smoke now fails if the server exports such an attribute.
