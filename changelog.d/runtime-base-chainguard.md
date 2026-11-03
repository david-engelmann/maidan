### The server and CLI images run on Chainguard's glibc-dynamic

- **Changed:** the server and `maidan` CLI images run on `cgr.dev/chainguard/glibc-dynamic`, pinned by digest, instead of `gcr.io/distroless/cc-debian12`. Distroless carried a `libssl3` (CVE-2026-84782) that the release scan refused for weeks after Debian's fix, so `v421.0.0` never published. The new base has glibc, libgcc, CA certificates and a `nonroot` user, and no shell and no OpenSSL. No Maidan binary links OpenSSL. The `.trivyignore` exception for it is gone.
