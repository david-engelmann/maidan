### url 2.5.9 for RUSTSEC-2026-0592

- **Security:** `url` moves from 2.5.8 to 2.5.9 (and `form_urlencoded` to 1.2.3 with it) in the root and fuzz lockfiles, for RUSTSEC-2026-0592, a stack overflow in `Url::origin()` on nested `blob:` URLs. The server's three calls to it are not reachable with one: a `blob:` redirect URI is refused at client registration, and the other two take `http` and `https` only.
