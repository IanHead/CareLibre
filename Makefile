SHELL := /bin/bash
.DEFAULT_GOAL := help
CARGO ?= cargo

.PHONY: help setup build run test fmt lint check docker-up docker-down docker-logs clean

help: ## Show this help
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

setup: ## Create .env from .env.example if missing
	@test -f .env || (cp .env.example .env && echo "created .env — set JWT_SECRET before deploying")

build: ## Compile the workspace
	$(CARGO) build --workspace

run: ## Run the server locally (requires PostgreSQL + .env)
	$(CARGO) run -p carelibre-server

test: ## Run unit and integration tests
	$(CARGO) test --workspace

test-db: ## Run schema integration tests (requires DATABASE_URL)
	$(CARGO) test --workspace -- --ignored

fmt: ## Format the code
	$(CARGO) fmt --all

lint: ## Run clippy with warnings as errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: fmt ## Format and lint everything
	$(CARGO) clippy --workspace --all-targets -- -D warnings

docker-up: ## Start PostgreSQL + server via docker compose
	docker compose up --build -d

docker-down: ## Stop the compose stack
	docker compose down

docker-logs: ## Tail compose logs
	docker compose logs -f server

clean: ## Remove build artefacts
	$(CARGO) clean
