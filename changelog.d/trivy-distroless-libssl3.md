### The release scan takes dated exceptions

- **Changed:** The release's Trivy scan reads `.trivyignore`, where each entry gives its reason and an expiry. The first entry is CVE-2026-84782, in the distroless base's `libssl3`, until 2026-11-02. No Maidan binary links OpenSSL, and distroless had not rebuilt with Debian's 3.0.22-1~deb12u2 fix when v418.0.0 was cut.
