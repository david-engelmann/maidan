### Check runs are not attempted with a GitHub token

- **Fixed:** With `MAIDAN_GITHUB_TOKEN` (no GitHub App), result delivery no longer tries to create a check run, which GitHub always refuses with 403. It records `skipped` and says so once at boot instead of a warning per result; the summary comment still carries the verdict.
