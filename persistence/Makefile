# ─── Detect container runtime ─────────────────────────────────────────────
DOCKER_COMPOSE := $(shell command -v podman-compose 2>/dev/null || echo docker compose)
COMPOSE_FILE   := docker/docker-compose.yml
ENV_FILE       := .env

# ANSI colours
GREEN  := \033[0;32m
YELLOW := \033[0;33m
RESET  := \033[0m

.PHONY: help up down build logs logs-app shell clean

help: ## Show this help message
	@echo "Usage: make <target>"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-18s$(RESET) %s\n", $$1, $$2}'

# ─── Environment check ────────────────────────────────────────────────────
check-env:
	@if [ ! -f $(ENV_FILE) ]; then \
		echo "$(YELLOW)Warning: $(ENV_FILE) not found. Copy .env.example to .env and edit it.$(RESET)"; \
		exit 1; \
	fi

# ─── Container targets ────────────────────────────────────────────────────
up: check-env ## Start all services (ScyllaDB + app)
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) up -d

down: ## Stop and remove containers
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) down

build: check-env ## (Re)build the app container image
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) build app

logs: ## Tail logs for all services
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) logs -f

logs-app: ## Tail logs for the persistence service only
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) logs -f app

shell: ## Open a shell inside the running app container
	$(DOCKER_COMPOSE) -f $(COMPOSE_FILE) exec app /bin/sh

# ─── Local dev targets ────────────────────────────────────────────────────
dev: ## Run locally with cargo (requires ScyllaDB running separately)
	cargo run

release: ## Build a release binary locally
	cargo build --release

test: ## Run tests
	cargo test

clean: ## Remove compiled artifacts
	cargo clean
