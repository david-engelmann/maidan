### The smoke jobs reuse a cached server image build

- **Changed:** The docker compose, scale-out and OTLP smoke jobs build the server image through buildx with a GitHub Actions layer cache, so the cargo-chef dependency layer is reused across runs instead of compiled cold in each job.
