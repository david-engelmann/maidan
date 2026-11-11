### Honest HTTP metrics and a protected `/metrics`

- **Security:** HTTP metrics use a closed method set (`GET`, `POST`, `PUT`,
  `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, or `other`) and a `route` label that is
  the matched-path template, never the raw URI, so a caller cannot mint
  unbounded series. The series are now `maidan_http_request_total` and
  `maidan_http_request_duration_seconds`. In production, `/metrics` is not on
  the public listen address unless `MAIDAN_METRICS_TOKEN` is set; otherwise the
  server listens on `0.0.0.0:9091` (override with `MAIDAN_METRICS_BIND`) and
  compose stays on the app port for development. A configured token requires
  `Authorization: Bearer` on whichever listener serves the path. Helm, k8s,
  Alloy, and the SLO alert rules follow the new names and scrape port.
