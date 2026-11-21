### The test containers no longer bring in tokio-tar

- **Security:** `testcontainers` moves from 0.23 to 0.27 (and `testcontainers-modules` from 0.11 to 0.15), which removes `tokio-tar` 0.3.1 from the lockfile. That crate has an open advisory with no fixed release (RUSTSEC-2025-0111, GHSA-j5gw-2vrg-8fgx) and was the repository's one open Dependabot alert. It was never in a shipped binary: only the integration tests reach it. `rustls-pemfile` leaves the lockfile with it.
- **Changed:** `deny.toml` drops the two advisory ignores that covered those crates and gains one for RUSTSEC-2026-0440, a notice that `astral-tokio-tar`, the replacement, is unmaintained. The locked 0.6.4 has no open vulnerability advisory and is test-only too. Three skips for `rand` 0.8 crates that matched nothing are gone.
- **Changed:** `supply-chain/` exempts the fifteen crate versions the bump brings in, unreviewed, with the reason on each.
