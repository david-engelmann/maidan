### Cargo caches are saved only from main and dev

- **Changed:** Every `Swatinem/rust-cache` step in `ci.yml` saves only on `main` and `dev`, as the lint job already did (#1680). PR-scoped saves of up to 1 GB each had filled the 10 GB Actions cache and evicted the `dev` entries that pull requests restore from, so most PR jobs compiled cold.
