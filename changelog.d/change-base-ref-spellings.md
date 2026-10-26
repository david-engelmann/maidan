### A change's base refuses every ref spelling and non-ASCII names

- **Fixed:** The change flow and the mark-ready base map now also refuse `HEAD` and bases starting `tags/`, `remotes/` or `origin/`, and a branch name must be ASCII, so a look-alike or zero-width character can't stand in for `prod` (#1690).
- **Fixed:** Blessing a `github_branch` base (`owner/name@base`) now refuses the same ref spellings, so a row that could never be used, such as `@heads/dev`, can no longer be added. Pseudo-refs such as `FETCH_HEAD` and `ORIG_HEAD`, and any other name ending in `_HEAD`, are refused as bases too.
- **Changed:** A stored branch target or blessing whose name is not ASCII is now refused rather than migrated. It stops decoding, nothing is delivered to it, and it must be re-added under an ASCII name.
