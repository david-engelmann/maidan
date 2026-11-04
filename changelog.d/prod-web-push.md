### maidan.world can send web push

- **Added:** The hosted Worker passes `MAIDAN_VAPID_PRIVATE_KEY`, `MAIDAN_VAPID_PUBLIC_KEY` and `MAIDAN_VAPID_SUBJECT` to the server, so once those secrets are set the console's This device can turn notifications on. Before, the hosted instance had no way to receive a VAPID key.
