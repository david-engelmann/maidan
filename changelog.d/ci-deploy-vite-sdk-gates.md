### CI deploy gates, vite CSP parity, and Python SDK export guards

- **Fixed:** `check-deploy-contract.sh` no longer depends on `rg` or swallows scan failures when checking MinIO tags and Chainguard digests.
- **Fixed:** `check-deploy-pins.sh` rejects non-numeric version inputs in `version_gt` and fails closed when `grep` errors inside `pin()`.
- **Fixed:** `osv-scan.sh` hash-verifies downloaded (and `OSV_SCANNER` override) binaries before `mv`/`chmod`.
- **Fixed:** `check-alert-rules.sh` refuses a missing rules CRD even when `promtool` is absent, and validates the extracted `groups` document.
- **Fixed:** `check-reviewer-token-expiry.sh` bounds `MAIDAN_REVIEW_WAIT_DAYS` and rejects non-http(s) or credentialed `MAIDAN_URL`s before any network call.
- **Changed:** Landing Vite builds set `assetsInlineLimit: 0` for strict-CSP parity with the console; console dev enables `fs.strict` / `strictPort` with loopback-only proxies.
- **Changed:** Python `maidan` package rejects non-semver `__version__` values and unknown attribute typos via `__getattr__`.
