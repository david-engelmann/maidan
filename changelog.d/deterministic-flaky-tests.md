### Two flaky CI tests no longer race their setup

- **Fixed:** the S3 tests wait for MinIO's readiness probe before their first request, instead of trusting its `API:` log line, which can come before it serves PUTs. The board's `tsc --checkJs` test runs TypeScript from the npm cache, which CI fills (with retries) before the test jobs, so a slow registry no longer runs it into nextest's 3-minute kill.
