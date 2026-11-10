### The hosted Worker recycles a container running on old settings

- **Fixed:** a deploy that changed a setting could leave the container running the new image on the previous settings, as `MAIDAN_SIGNUP_OPEN` did in v445 until the container slept. The Worker now starts the container with a hash of its settings (`MAIDAN_ENV_FINGERPRINT`). The server echoes it on every response as `x-maidan-env-fingerprint`, and a container that answers with another hash is recycled on the next request. A self-hosted server sets no fingerprint and sends no header.
