# =============================================================================
# DyingStar Resources Dynamic Service - Makefile
# =============================================================================

# Colors for output
CYAN := \033[36m
GREEN := \033[32m
YELLOW := \033[33m
RED := \033[31m
RESET := \033[0m

EXECUTOR := docker compose

# if podman is available, use it instead of docker
ifneq (, $(shell which podman 2>/dev/null))
$(info ❗❗ Using podman for compose commands❗❗)
EXECUTOR := podman compose
endif

# Get absolute path to project root
ROOT_DIR := $(shell pwd)
COMPOSE := $(EXECUTOR) -f $(ROOT_DIR)/docker/docker-compose.yml --env-file $(ROOT_DIR)/.env

DEV_SERVICE := dev
APP_SERVICE := app


# =============================================================================
# Check .env file exists
# =============================================================================
.PHONY: check-env
check-env:
	@if [ ! -f .env ]; then \
		echo "$(RED)❌ .env file not found!$(RESET)"; \
		echo "$(YELLOW)💡 Copy .env.example to .env and configure it:$(RESET)"; \
		echo "   cp .env.example .env"; \
		exit 1; \
	fi

# =============================================================================
# CP/PO/Others Profile - Simple site testing
# =============================================================================

.PHONY: start
start: check-env ## Start the complete application (install, build, start) for testing
	@echo "$(CYAN)🚀 Starting DyingStar Resources Dynamic Service for testing...$(RESET)"
	@echo "$(YELLOW)This will install dependencies, build, and start the application$(RESET)"
	@$(COMPOSE) up $(APP_SERVICE) --build

.PHONY: stop
stop: ## Stop all running services
	@echo "$(CYAN)🛑 Stopping all services...$(RESET)"
	@$(COMPOSE) down

# =============================================================================
# Dev Profile - Development commands
# =============================================================================

.PHONY: up
up: check-env ## Start only MeiliSearch for development (no app)
	@echo "$(CYAN)🔧 Starting development environment...$(RESET)"
	@$(COMPOSE) up $(DEV_SERVICE) -d
	@echo "$(GREEN)✅ Development environment ready!$(RESET)"
	@echo "$(YELLOW)Use 'make pnpm <command>' to run pnpm commands$(RESET)"

.PHONY: down
down: ## Stop development environment
	@echo "$(CYAN)🛑 Stopping development environment...$(RESET)"
	@$(COMPOSE) down

# Generic pnpm command runner
.PHONY: pnpm
pnpm: check-env ## Run any pnpm command (usage: make pnpm install, make pnpm dev, etc.)
	@if [ -z "$(filter-out $@,$(MAKECMDGOALS))" ]; then \
		echo "$(RED)❌ Please provide a pnpm command. Usage: make pnpm <command>$(RESET)"; \
		echo "$(YELLOW)Examples: make pnpm install, make pnpm dev, make pnpm build$(RESET)"; \
		exit 1; \
	fi
	@echo "$(CYAN)📦 Running: pnpm $(filter-out $@,$(MAKECMDGOALS))$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm $(filter-out $@,$(MAKECMDGOALS))"


# =============================================================================
# Database commands
# =============================================================================

.PHONY: db-shell
db-shell: check-env ## Open psql shell in PostgreSQL
	@echo "$(CYAN)🐘 Opening PostgreSQL shell...$(RESET)"
	@$(COMPOSE) exec postgres psql -U $(shell grep POSTGRES_USER .env | cut -d '=' -f2) -d $(shell grep POSTGRES_DB .env | cut -d '=' -f2)

.PHONY: db-logs
db-logs: ## Show PostgreSQL logs
	@$(COMPOSE) logs -f postgres

.PHONY: db-migrate
db-migrate: check-env ## Apply Drizzle migrations
	@echo "$(CYAN)🔄 Applying database migrations...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm db:migrate"
	@echo "$(GREEN)✅ Migrations applied$(RESET)"

.PHONY: db-push
db-push: check-env ## Apply Drizzle migrations push
	@echo "$(CYAN)🔄 Applying database push migrations...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm db:push"
	@echo "$(GREEN)✅ Migrations push applied$(RESET)"

.PHONY: db-studio
db-studio: check-env ## Open Drizzle Studio
	@echo "$(CYAN)🎨 Starting Drizzle Studio...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm db:studio"

.PHONY: db-status
db-status: check-env ## Check database connection status
	@echo "$(CYAN)🔍 Checking database status...$(RESET)"
	@$(COMPOSE) exec postgres pg_isready -U $(shell grep POSTGRES_USER .env | cut -d '=' -f2)

.PHONY: db-import-system
db-import-system: check-env ## Import JSON data from data/system/ folder
	@echo "$(CYAN)📥 Importing JSON data from data/system/ folder...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm import-system"

.PHONY: db-reset
db-reset: check-env ## Reset the database (⚠️ WARNING: This will delete all data! ⚠️)
	@echo "$(CYAN)💥 Resetting database...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "corepack enable && corepack install && pnpm reset-db"

.PHONY: db-full-reset
db-full-reset: check-env ## Reset the database, apply Drizzle migrations and import the JSON data from data/system/ folder
	@read -p "$$CYAN ⚠️  Are you sure you want to reset the database? (yes/no): $$RESET" confirm && \
	if [ "$$confirm" = "yes" ]; then \
		$(MAKE) db-reset; \
		$(MAKE) db-migrate; \
		$(MAKE) db-import-system; \
		echo "$$GREEN 🎉 Database completely reset!$$RESET"; \
	else \
		echo "$$CYAN 🚫 Operation canceled.$$RESET"; \
	fi

# =============================================================================
# Utility commands
# =============================================================================

.PHONY: logs
logs: ## Show logs for all services
	@$(COMPOSE) logs -f

.PHONY: logs-app
logs-app: ## Show logs for app service only
	@$(COMPOSE) logs -f $(APP_SERVICE)

.PHONY: logs-dev
logs-dev: ## Show logs for dev service only
	@$(COMPOSE) logs -f $(DEV_SERVICE)

.PHONY: shell
shell: ## Open shell in dev container
	@echo "$(CYAN)🐚 Opening shell in dev container...$(RESET)"
	@$(COMPOSE) exec $(DEV_SERVICE) sh

.PHONY: status
status: ## Show status of all services
	@echo "$(CYAN)📊 Service Status:$(RESET)"
	@$(COMPOSE) ps

.PHONY: clean-volumes
clean-volumes: ## Remove all volumes (WARNING: This will delete all data!)
	@echo "$(RED)⚠️  This will delete all Docker volumes and data!$(RESET)"
	@read -p "Are you sure? (y/N) " answer; \
	if [ "$$answer" = "y" ] || [ "$$answer" = "Y" ]; then \
		$(COMPOSE) down -v; \
		echo "$(GREEN)✅ Volumes cleaned$(RESET)"; \
	else \
		echo "$(YELLOW)Cancelled$(RESET)"; \
	fi

.PHONY: env-check
env-check: check-env ## Verify .env configuration
	@echo "$(CYAN)🔍 Environment Configuration:$(RESET)"
	@echo "$(YELLOW)POSTGRES_USER:$(RESET)     $(shell grep POSTGRES_USER .env | cut -d '=' -f2)"
	@echo "$(YELLOW)POSTGRES_DB:$(RESET)       $(shell grep POSTGRES_DB .env | cut -d '=' -f2)"
	@echo "$(YELLOW)POSTGRES_PORT:$(RESET)     $(shell grep POSTGRES_PORT .env | cut -d '=' -f2)"
	@echo "$(YELLOW)WS_PORT:$(RESET)           $(shell grep WS_PORT .env | cut -d '=' -f2)"
	@echo "$(YELLOW)API_PORT:$(RESET)          $(shell grep API_PORT .env | cut -d '=' -f2)"
	@echo "$(YELLOW)NODE_ENV:$(RESET)          $(shell grep NODE_ENV .env | cut -d '=' -f2)"


# =============================================================================
# Help
# =============================================================================

.PHONY: help
help: ## Show this help message
	@echo "$(CYAN)DyingStar Resources Dynamic - Available Commands:$(RESET)"
	@echo ""
	@echo "$(YELLOW)CP/PO/Others Profile (Simple Testing):$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(start|stop)"
	@echo ""
	@echo "$(YELLOW)Dev Profile (Development):$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(up|down|pnpm)"
	@echo ""
	@echo "$(YELLOW)Utilities:$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(logs|shell|status|clean-volumes)"
	@echo ""
	@echo "$(YELLOW)Database:$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(db)"
	@echo ""
	@echo "$(YELLOW)Examples:$(RESET)"
	@echo "  $(CYAN)make start$(RESET)          # Start app for testing (CP/PO/Others)"
	@echo "  $(CYAN)make up$(RESET)             # Start dev environment (Dev)"
	@echo "  $(CYAN)make pnpm dev$(RESET)       # Start dev server"
	@echo "  $(CYAN)make pnpm build$(RESET)     # Build the application"
	@echo "  $(CYAN)make install$(RESET)        # Install dependencies"

# Default target
.DEFAULT_GOAL := help

# Allow arguments to be passed to make commands
%:
	@: