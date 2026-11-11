### GitHub Actions are pinned to commit SHAs

- **Security:** every `uses:` in `.github/workflows` is pinned to a full
  commit SHA, with the version tag or branch kept in a comment. The lint
  job fails if an action is unpinned. Dependabot opens weekly grouped
  updates for GitHub Actions, cargo, npm and Docker.
