### A second-family review is served where its change is

- **Fixed:** The review child Maidan spawns when a change enters review now carries the change's `engine`, `repo` and `lane`. It carried none, so a seat that claims or reads queue depth filtered by engine and repository never counted or claimed it, and reviews waited unclaimed.
- **Changed:** `claim_next`'s and queue depth's `repo` filter matches a thread whose footprint or `repo` attribute names the repository. A review child holds no footprint: it reads and never writes, so no claim waits on it.
- **Fixed:** Migration 0230, a one-time backfill: an open or in-review review child with no footprint takes its change's `engine` and `repo` (from the change's attributes, else its footprint) where it has none. A closed review is left alone, and a second run changes nothing.
- **Changed:** The docs, the reviewer and red-team skills and the `review_family` descriptions say what is true: any member whose usage never named a review's `author_family` may claim it, `review_family` is a label nothing routes on, and a review seat filters on `kind=review`, never on `review_family`.
