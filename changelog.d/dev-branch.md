### Work lands on `dev`; `main` is prod

- **Changed:** `dev` is the default branch. Every pull request targets it, with the same eight required checks, and CI and the docs build run on pushes to both `main` and `dev`. dev.maidan.world deploys `dev` (images are tagged `dev-<sha>`). `main` moves only when a tested `dev` commit is fast-forwarded into it, and is then tagged and deployed to maidan.world.
- **Security:** the OpenHands example's `litellm` floor moves to 1.100.4, the fixed release for GHSA-7hp6-4w63-5g45 (a proxy-admin privilege escalation). litellm pins `filelock<4.0`, and its three advisories (fixed only in 4.2.1) are accepted for that example in `.config/osv-scanner.toml`, with the reason.
