### A migration file is its own registration

- **Changed:** `maidan-store` builds its migration register from `migrations/{postgres,sqlite}` at compile time. Every `NNNN_<slug>.sql` (except `*_down.sql`) is applied as version `NNNN`, in order, replacing the 371-line hand list in `migrate.rs`. A name without a four-digit version, or two files with one version, fails the build. The registered set and order are unchanged, and so is the chain backfill after pg 91 / sqlite 90.
