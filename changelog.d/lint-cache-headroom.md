### The lint job starts warm and has headroom

- **Fixed:** the `lint (fmt + clippy + deny)` job restores one shared cache saved from `main` and `dev`, so a pull request no longer compiles all 716 crates from scratch, and its timeout moves from 15 to 25 minutes for a cold run on a slow runner (#1680). PRs had been timing out and reporting the required check as cancelled.
