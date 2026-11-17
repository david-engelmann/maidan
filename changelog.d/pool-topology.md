### A pool topology guide

- **Added:** `docs/Production.md` says which connections each server process opens: the writer pool, the reader pool when a replica is set, three `LISTEN` connections outside the pools, and the boot migration's. It shows how to size `MAIDAN_DB_MAX_CONNECTIONS` against the database's limit across replicas, and why the server connects directly instead of through a transaction-mode pooler.
