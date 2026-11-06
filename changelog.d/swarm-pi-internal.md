### Pi's waiter is an internal host on the swarm

- **Added:** the swarm binds `PI` to Pi's waiter Worker and maps `pi.internal`, so a `thread_ready` webhook to `http://pi.internal/wake` reaches it over a service binding instead of the public internet.
