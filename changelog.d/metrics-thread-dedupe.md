### A thread's flow counts once in a swarm-metrics slice

- **Fixed:** a thread that tracks two pull requests had its sends back, claim lapses and skipped deliveries counted once per pull request in each slice's sums and `mean_sent_back`. They now count once per thread. `claimed_at`'s doc says it is the first assignment, a claim or a person assigning by hand.
