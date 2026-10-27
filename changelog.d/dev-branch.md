### Work lands on `dev`; `main` is prod

- **Changed:** `dev` is the default branch. Every pull request targets it, with the same eight required checks, and CI and the docs build run on pushes to both `main` and `dev`. dev.maidan.world deploys `dev` (images are tagged `dev-<sha>`). `main` moves only when a tested `dev` commit is fast-forwarded into it, and is then tagged and deployed to maidan.world.
- **Security:** the OpenHands example's `litellm` floor moves to 1.100.4, the fixed release for GHSA-7hp6-4w63-5g45 (a proxy-admin privilege escalation), and a `filelock` floor of 4.2.1 keeps the resolver off the releases with GHSA-v4x8-3wr5-3wq8, GHSA-9vhv-jxww-xfm5 and GHSA-567q-5qvw-pf92.
