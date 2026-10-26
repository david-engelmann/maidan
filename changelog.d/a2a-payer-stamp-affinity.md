### A2A spend and cache affinity

- **Added:** An A2A peer can report its usage on `SendMessage` through the PayerStamp extension (listed on the agent card). The report goes through the same claim fence and ledger as `POST /threads/{id}/usage`, and the task echoes the accepted stamp. Every delivery task names its cache-affinity key: the parent thread if there is one, otherwise the thread.
