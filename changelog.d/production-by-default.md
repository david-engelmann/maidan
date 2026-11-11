### Production is the default environment

- **Security:** `MAIDAN_ENV` accepts only `production` and `development`.
  Unset is `production`, and any other value refuses boot. The Helm
  production values and the Kubernetes prod overlay set
  `MAIDAN_ENV=production`. Compose, the quickstart and the chart's
  development values set `development`. The OIDC mock cannot run in
  production.
