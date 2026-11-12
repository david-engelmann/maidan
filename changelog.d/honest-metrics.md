### HTTP metrics that a client cannot inflate, behind a token in production

- **Security:** the HTTP metrics label the method from a fixed set (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, `other`). They used the raw method, so 200 requests with random extension methods added 2,000 series, which the recorder kept for the life of the process.
- **Added:** `http_server_request_total` and `http_server_request_duration_seconds` carry a `route` label: the matched route template (`/threads/{id}/result`), or `unmatched`, never the path.
- **Security:** `MAIDAN_METRICS_TOKEN` (32 bytes or more, with a `_FILE` form) puts `GET /metrics` behind that bearer. Under `MAIDAN_ENV=production` without it, `/metrics` answers `404`; it used to answer anyone on the app port. Outside production it stays open, as before. The Cloudflare Worker forwards the token, the Helm chart can mount it as a secret file, and the Alloy workbench scrape sends it.
