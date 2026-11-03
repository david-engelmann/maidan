### The console's corners and review colour come from tokens

- **Changed:** Cards, sheets, chips and buttons take their corners from one radius scale (`--r-xs` through `--r-pill`) instead of 14 loose pixel values, so the surfaces read as one set.
- **Changed:** Review states and review-request badges use a `--review` colour token with a dark-mode twin, replacing the hard-coded fallback.
