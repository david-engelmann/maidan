### Swarm metrics split by review tier

- **Added:** each repository rollup in the swarm metrics carries `by_tier`, the tasks of review tier `A` and of tier `B`, with a task that has no tier stored counted as `B`. A tier with no tasks is left out, as in the other splits.
