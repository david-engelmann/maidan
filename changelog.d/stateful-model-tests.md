### Thread transitions follow a store model

- **Added:** `thread_transition_state_machine` runs random sequences of thread lifecycle actions (`start_review`, `request_changes`, `close`, `archive`), including a parent/child pair, against a small model and the real SQLite and Postgres stores. After every step each thread's state and transition log match the model. An illegal action, or one that would put a child ahead of its parent, changes nothing.
