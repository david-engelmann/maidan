### A change's base refuses every ref spelling and non-ASCII names

- **Fixed:** The change flow and the mark-ready base map now also refuse `HEAD` and bases starting `tags/`, `remotes/` or `origin/`, and a branch name must be ASCII, so a look-alike or zero-width character can't stand in for `prod` (#1690).
