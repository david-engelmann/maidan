### Prod deploys only what main carries

- **Fixed:** the `deploy prod` workflow refuses a release tag whose commit is not on `main`, and its image push reads `wrangler.prod.jsonc`. It read the dev Wrangler config, whose Dockerfile build is not in the checkout, so the push failed. The `prod` GitHub environment allows only `main` and `v*` tags.
