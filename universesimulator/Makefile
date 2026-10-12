.PHONY: build run test fmt lint clean migrate migrate-revert docker-up docker-down docker-logs db-up import-seed tick

# Test database (sqlx::test creates one database per test).
TEST_DATABASE_URL ?= postgres://universesimulator:universesimulator@localhost:5434/universesimulator
ADMIN_HEADERS = -H 'X-Player-Id: 00000000-0000-4000-8000-00000000ad01' -H 'X-Player-Roles: admin'

# ── Local development ────────────────────────────────────────────────────────

build:
	cargo build

run:
	cargo run

# Needs PostgreSQL (make db-up).
test:
	DATABASE_URL=$(TEST_DATABASE_URL) cargo test

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

# ── Database helpers ─────────────────────────────────────────────────────────

# Apply pending migrations (requires DATABASE_URL in environment or .env).
migrate:
	sqlx migrate run --source migrations

# Revert last migration.
migrate-revert:
	sqlx migrate revert --source migrations

# ── Dev helpers (AUTH_DEV_BYPASS=true) ───────────────────────────────────────

import-seed:
	curl -sf -X POST http://localhost:9600/api/admin/import $(ADMIN_HEADERS) \
		-H 'Content-Type: application/json' --data @seeds/tarsis_3-ares.json

tick:
	curl -sf -X POST http://localhost:9600/api/admin/simulation/tick $(ADMIN_HEADERS)

# ── Docker ───────────────────────────────────────────────────────────────────

db-up:
	docker compose -f docker/docker-compose.yml up -d postgres

docker-up:
	docker compose -f docker/docker-compose.yml up -d

docker-down:
	docker compose -f docker/docker-compose.yml down

docker-logs:
	docker compose -f docker/docker-compose.yml logs -f
