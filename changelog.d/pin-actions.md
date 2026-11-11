### Every GitHub Action pinned to a commit

- **Security:** every `uses:` in `.github/workflows/` names a full commit SHA with its version as a trailing comment, instead of a tag or branch its owner could move (`trufflesecurity/trufflehog@main` and `dtolnay/rust-toolchain@master` included); the secrets scan also pins its scanner image to 3.99.8. `scripts/check-action-pins.sh` in the `lint` job fails on any unpinned `uses:`.
- **Security:** the `maidan-postgres` and quickstart images build from base images pinned by digest (`pgvector/pgvector:pg16`, `ubuntu:24.04`).
- **Added:** `.github/dependabot.yml` opens one grouped pull request a week each for GitHub Actions, Cargo, the npm lockfiles that a build, deploy or CI job installs from, and the Dockerfiles' base images.
