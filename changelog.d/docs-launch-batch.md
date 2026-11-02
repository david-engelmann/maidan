### Docs: quickstart and README fixes from a first-run audit

- **Fixed:** The README now says to set `MAIDAN_URL` along with `MAIDAN_HOST_PORT` when port 8080 is taken; otherwise the quickstart script still calls port 8080.
- **Fixed:** The from-source steps list their prerequisites (Rust toolchain, `curl`, `jq`) and wait for `/health` before `init` and the demo script.
- **Fixed:** The Postgres compose example starts detached with `--wait`, so the `curl` after it works in the same shell.
- **Fixed:** The mdBook install pins `0.4.40`, the version CI builds with.
