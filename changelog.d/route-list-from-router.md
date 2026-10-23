### The live-stream route check classifies routes instead of counting them

- **Changed:** `tenant_isolation_e2e` no longer pins the number of `.route(` calls in `app.rs` (386). It reads every routed path and requires each one to be either an OpenAPI operation or listed in `tests/fixtures/non_openapi_routes.json` as a stream or not a stream. A route outside the spec fails until it is classified, and a new stream fails until `stream_probes` covers it. Adding an ordinary OpenAPI route no longer touches the test.
