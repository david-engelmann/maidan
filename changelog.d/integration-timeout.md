### The integration job has room for the whole suite

- **Fixed:** the required `integration (testcontainers)` job's limit moves from 40 to 60 minutes. With 3,353 tests, a run on a slow runner hit 40 minutes and was cancelled with nothing failed, which held every `dev` commit at the swarm's deploy gate.
