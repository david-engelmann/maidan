### CI/nightly credential pins and Cloudflare wake cost gates

- **Fixed:** Every `actions/checkout` in `ci.yml` and `nightly.yml` sets `persist-credentials: false`.
- **Fixed:** `release-darwin-x86.yml` refuses non-`vN.N.N` tags, pins checkout credentials off, and uses `pipefail`.
- **Added:** `scripts/check-ci-nightly-wrangler-gates.sh` locks those pins and the once-daily Cloudflare container wake cron.
- **Fixed:** `sdk-test.sh` refuses unsupported languages and out-of-range ports.
