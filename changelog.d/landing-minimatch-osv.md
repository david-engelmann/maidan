### The landing page's build tools no longer pull a vulnerable minimatch

- **Security:** `web/landing` pins `minimatch` to 11.0.3 through an npm `overrides` entry, clearing GHSA-g3hv-75wg-m249 (denial of service through glob-part amplification) from the dev dependencies that eslint and typescript-eslint pulled in at 3.1.5 and 9.0.9. No release line below 11.0.1 has a fix. Only dev tooling changes; the built landing page is byte-for-byte the same.
