# =============================================================================
# DyingStar Planet Service - Makefile
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

COMPOSE := $(EXECUTOR) -f docker/docker-compose.yml --env-file .env

DEV_SERVICE := dev
PYTHON := $(COMPOSE) exec ${DEV_SERVICE} python

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
# Dev Profile - Development commands
# =============================================================================

.PHONY: up
up: check-env ## Start only python for development
	@echo "$(CYAN)🔧 Starting development environment...$(RESET)"
	@$(COMPOSE) up $(DEV_SERVICE) -d
	@echo "$(GREEN)✅ Development environment ready!$(RESET)"
	@echo "$(YELLOW)Use 'make pnpm <command>' to run pnpm commands$(RESET)"

.PHONY: down
down: ## Stop development environment
	@echo "$(CYAN)🛑 Stopping development environment...$(RESET)"
	@$(COMPOSE) down

# =============================================================================
# Script commands
# =============================================================================

.PHONY: init-object-database
init-object-database: ## Insert object in database
	@echo "$(CYAN)🔄 Insert object in database...$(RESET)"
	@$(PYTHON) import_system_from_json.py ./systems/tarsis.json Tarsis


.PHONY: generate-kepler-positions
generate-kepler-positions: ## Insert object in database
# 	@if [ -z "$(filter-out $@,$(MAKECMDGOALS))" ]; then \
# 		echo "$(RED)❌ Please provide a kepler_positions command. Usage: make generate-kepler-positions <command>$(RESET)"; \
# 			$(PYTHON) kepler_positions.py -h; \
# 		exit 1; \
# 	fi
# 	@echo "$(CYAN)📦 Running: kepler_positions.py $(filter-out $@,$(MAKECMDGOALS))$(RESET)"
	@echo "$(CYAN)🔄 Insert kepler position in database...$(RESET)"
# 	@$(COMPOSE) exec $(DEV_SERVICE) sh -c "python kepler_positions.py $(filter-out $@,$(MAKECMDGOALS))"
# 	@$(PYTHON) kepler_positions.py --type planet --id 3 --duration 86400 --no-json
# 	@$(PYTHON) kepler_positions.py --type planet --id 4 --duration 86400 --no-json
# 	@$(PYTHON) kepler_positions.py --type planet --id 5 --duration 86400 --no-json
# 	@$(PYTHON) kepler_positions.py --type planet --id 6 --duration 86400 --no-json
# 	@$(PYTHON) kepler_positions.py --type planet --id 7 --duration 86400 --no-json
# 	@$(PYTHON) kepler_positions.py --type planet --id 8 --duration 86400 --no-json


# 	@$(PYTHON) kepler_positions.py -h

# =============================================================================
# Utility commands
# =============================================================================

.PHONY: logs
logs: ## Show logs for all services
	@$(COMPOSE) logs -f

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

.PHONY: env-check
env-check: check-env ## Verify .env configuration
	@echo "$(CYAN)🔍 Environment Configuration:$(RESET)"
	@echo "$(YELLOW)PGHOST:$(RESET) $(shell grep PGHOST .env | cut -d '=' -f2)"
	@echo "$(YELLOW)PGPORT:$(RESET) $(shell grep PGPORT .env | cut -d '=' -f2)"
	@echo "$(YELLOW)PGDATABASE:$(RESET) $(shell grep PGDATABASE .env | cut -d '=' -f2)"
	@echo "$(YELLOW)PGUSER:$(RESET) $(shell grep PGUSER .env | cut -d '=' -f2)"
	@echo "$(YELLOW)PGPASSWORD:$(RESET) $(shell grep PGPASSWORD .env | cut -d '=' -f2)"

# =============================================================================
# Help
# =============================================================================

.PHONY: help
help: ## Show this help message
	@echo "$(CYAN)DyingStar Planet Service - Available Commands:$(RESET)"
	@echo ""
	@echo "$(YELLOW)CP/PO/Others Profile (Simple Testing):$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(start|stop)"
	@echo ""
	@echo "$(YELLOW)Dev Profile (Development):$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(up|down)"
	@echo ""
	@echo "$(YELLOW)Utilities:$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(CYAN)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST) | grep -E "(logs|shell|status|clean-volumes)"
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