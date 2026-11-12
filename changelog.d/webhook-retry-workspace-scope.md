### A retried webhook delivery stays in its workspace

- **Security:** a queued webhook delivery is retried only for an event in its hook's own workspace; one carrying another workspace's event is quarantined unsent.
