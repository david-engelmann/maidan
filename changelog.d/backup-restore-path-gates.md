### Fixed

- **Fixed:** `backup.sh` / `restore.sh` refuse backup dirs and artifact roots that are system roots, contain `..`, or are too shallow before mkdir/tar; `check-changelog-released` validates the compared version is semver before `git show`.
