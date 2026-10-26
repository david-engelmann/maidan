### An outside approval's submitter must belong to the thread's workspace

- **Fixed:** When a review was submitted on a reviewer's behalf, the outside approval that lets a past holder close now also requires the submitter to be a member of the thread's own workspace, on SQLite and Postgres. Tests now cover a removed submitter and a removed land-gate recorder (#1685).
