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

compose-up:
	docker compose up -d

compose-down:
	docker compose down -v

db-up:
	docker compose up -d postgres

db-down:
	docker compose stop postgres

smoke: compose-up
	@echo "waiting for /health..."
	@for i in 1 2 3 4 5 6 7 8 9 10; do \
		curl -sf http://localhost:8080/health && echo "" && exit 0; \
		sleep 2; \
	done; \
	echo "health check failed"; exit 1

clean:
	cargo clean
	rm -rf .local data logs
