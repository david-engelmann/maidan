### try.maidan.world deploys from each release, and costs less to run

- **Added:** each release publishes `maidan-server:<tag>-demo`. It is built with the bootstrap routes for the try.maidan.world seed, and is signed, SBOM-attested and trivy-scanned like the stock image. It is amd64 only and never tagged `latest`.
- **Added:** `.github/workflows/deploy-demo.yml` runs on a published release or on a dispatch, and only when `vars.DEMO_DOMAIN` is `true`. It cosign-verifies the demo server and CLI images, builds the demo image on them without compiling anything, pushes it with `wrangler containers push`, and deploys `wrangler.demo.jsonc`.
- **Changed:** the demo sleeps after 15 idle minutes instead of 30. Its reset cron runs once a day instead of every 6 hours, and only destroys a running container, so it never wakes one. Seeded tokens now last 30 hours. `deploy/cloudflare/test/cost.test.js` checks all of this.
- **Changed:** `scripts/check-deploy-contract.sh` fails if a stock server image stops building with `MAIDAN_ENABLE_BOOTSTRAP=0`, or if the demo image is tagged `latest`.
- **Docs:** `docs/Deploy.md` has a try.maidan.world section.
