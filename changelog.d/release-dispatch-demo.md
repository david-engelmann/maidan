### A published release starts the demo deploy

- **Fixed:** `release.yml` dispatches `deploy-demo.yml` and `clean-install.yml` after it publishes, the same way it already starts `deploy-prod.yml`. A release created with `GITHUB_TOKEN` does not trigger `release: published`, so try.maidan.world and the clean-machine install never ran on their own. The demo workflow still deploys only when `DEMO_DOMAIN` is `true`. A server release does not publish the SDKs, and the docs site still deploys on a push to `main`.
