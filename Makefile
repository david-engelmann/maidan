.PHONY: ci fmt lint test build deny secrets compose-up compose-down db-up db-down migrate clean smoke

ci: fmt lint deny test

fmt:
	cargo fmt --check

lint:
	cargo clippy --all-targets --workspace -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

deny:
	cargo deny check

secrets:
	@command -v trufflehog >/dev/null 2>&1 || { echo "trufflehog not installed; skipping"; exit 0; }
	trufflehog filesystem . --no-update --fail

# The server lives in the `full` profile; without it this starts Postgres only.
compose-up:
	docker compose --profile full up -d

compose-down:
	docker compose --profile full down -v

db-up:
	docker compose up -d postgres

db-down:
	docker compose stop postgres

# Brings the stack up and waits for the server to answer. MAIDAN_HOST_PORT
# moves the published port when 8080 is taken.
#
# `--wait` returns when every service reports healthy, so the polling loop this
# used to hand-roll now lives in the compose health checks, where `docker ps`
# and `depends_on` can see it too.
smoke:
	@if ! docker compose --profile full up -d --wait; then \
		echo "stack did not become healthy — recent server logs:"; \
		docker compose --profile full logs --tail 40 maidan-server || true; \
		exit 1; \
	fi
	@port=$${MAIDAN_HOST_PORT:-8080}; \
	curl -sf "http://localhost:$$port/health" && echo ""

clean:
	cargo clean
	rm -rf .local data logs
