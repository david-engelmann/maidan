### `prod` behind a remote prefix is never a change base

- **Fixed:** a change pull request's base and a `github_branch` allowlist selector now refuse `prod` behind any remote-style prefix (`upstream/prod`, `fork/PROD`), not only bare `prod` (#1693).
- **Changed:** documented that a `github_branch` row stored with a non-ASCII branch before #1691 stays listable for revocation but no longer parses as a target (#1693).
