### The release installs Trivy from its GitHub release

- **Fixed:** The release workflow's image scans and SBOMs install Trivy from its GitHub release, checked against the published checksums, rather than through `setup-trivy`. That installer downloads from `get.trivy.dev`, which stopped resolving on 2026-11-03 and failed every scan of the v424 release.
