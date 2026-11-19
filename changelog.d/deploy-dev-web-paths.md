### dev.maidan.world follows changes to the landing page and the console

- **Fixed:** the `deploy dev` workflow ran only when `crates/`, the migrations, the lockfile or the deploy itself changed. The landing page and the console under `web/` are built into the server, so a change to either merged to `dev` and was not deployed until some later server change (two landing changes sat undeployed on 2026-11-18). The workflow now runs on `web/**` too.
