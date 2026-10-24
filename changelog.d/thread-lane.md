### A thread can run in the batch or flex lane

- **Added:** a `lane` task attribute (Context Economics C7a): `batch` or `flex`, the service lane a thread may run in. Unset runs at standard rates. It can be set on thread create, `PUT /threads/{id}/attributes`, and the MCP `create_thread` and `set_thread_attributes`, and it is carried on live claims and the change metrics rows. Claim routing by lane follows in C7b. Migration 0205 adds a nullable `lane` column to `maidan_thread_attributes` and changes no existing data.
