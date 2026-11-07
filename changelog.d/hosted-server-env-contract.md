### The hosted container receives the secrets it boots with

- **Fixed:** The Worker forwards the secrets and rotation companions the hosted server actually reads, including `FEDERATION_DECRYPT_KEYS`, `MAIDAN_CONTENT_KEK_PREVIOUS`, `MAIDAN_EXPORT_VERIFY_KEYS`, and the Slack signing secret. A test refuses a Worker name the server does not know, and a secret the server reads that the Worker would drop. Dev and insecure switches are not forwarded.
