### The hosted Worker passes the GitHub token to the server

- **Fixed:** the hosted Worker forwards `MAIDAN_GITHUB_TOKEN` and `MAIDAN_GITHUB_WRITE_REPOS` to the server. They were not on its `SERVER_ENV` list, so a hosted instance could never commit a session's change, open its draft pull request or post a review on one: every GitHub delivery failed.
