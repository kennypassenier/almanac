# =============================================================================
# Makefile — cal-stacean (Smart Doppler Automation)
# =============================================================================

# -----------------------------------------------------------------------------
# Structural variables
# -----------------------------------------------------------------------------

BINARY_NAME   = cal-stacean
ENV_FILE      = .env
ENV_EXAMPLE   = .env.example

REGISTRY      = ghcr.io
GH_USERNAME   = kennypassenier
IMAGE_NAME    = cal-stacean
IMAGE_TAG     = v0.1.0

FULL_IMAGE    = $(REGISTRY)/$(GH_USERNAME)/$(IMAGE_NAME):$(IMAGE_TAG)

.PHONY: all secrets example-env build run clean \
        docker-build docker-login docker-push help


# -----------------------------------------------------------------------------
# Default target
# -----------------------------------------------------------------------------
all: secrets build


# -----------------------------------------------------------------------------
# help — print a concise target reference
# -----------------------------------------------------------------------------
help:
	@echo ""
	@echo "cal-stacean — available make targets"
	@echo "-------------------------------------"
	@echo "  secrets        Fetch secrets from Doppler and write $(ENV_FILE)"
	@echo "  example-env    Generate $(ENV_EXAMPLE) key template (values stripped)"
	@echo "  build          Compile release binary and copy to project root"
	@echo "  run            Run binary with Doppler-injected secrets"
	@echo "  clean          Remove build artefacts and local env files"
	@echo ""
	@echo "  docker-build   Build the Docker image and tag it for GHCR"
	@echo "  docker-login   Authenticate against ghcr.io"
	@echo "  docker-push    Build the image and push it to GHCR"
	@echo ""
	@echo "  FULL_IMAGE = $(FULL_IMAGE)"
	@echo ""


# =============================================================================
# LOCAL MANAGEMENT TARGETS
# =============================================================================

secrets:
	@echo "Fetching secrets from Doppler..."
	@doppler secrets download --format=env --no-file > $(ENV_FILE)
	@echo "$(ENV_FILE) successfully generated."

example-env:
	@echo "Generating $(ENV_EXAMPLE) template..."
	@doppler secrets download --format=env --no-file \
		| sed 's/=.*$$/=your_value_here/' > $(ENV_EXAMPLE)
	@echo "$(ENV_EXAMPLE) successfully generated."

build: secrets example-env
	@echo "Building Rust binary in release mode..."
	@cargo build --release
	@cp target/release/$(BINARY_NAME) ./$(BINARY_NAME)
	@echo "Build complete. Binary placed at: ./$(BINARY_NAME)"

# Smart wrapper for run: automatically prefix with doppler run if not already wrapped
run: build
	@echo "Starting daemon..."
	@if [ -z "$$DOPPLER_PROJECT" ]; then \
		echo "Doppler environment not detected, wrapping execution with 'doppler run'..."; \
		doppler run -- ./$(BINARY_NAME); \
	else \
		./$(BINARY_NAME); \
	fi


# =============================================================================
# CONTAINER PIPELINE TARGETS
# =============================================================================

docker-build:
	@echo "Building Docker image: $(FULL_IMAGE)"
	docker build \
		--tag $(FULL_IMAGE) \
		.
	@echo "Docker image built and tagged: $(FULL_IMAGE)"

# Smart wrapper for login: fetches CR_PAT via Doppler on-the-fly if missing
docker-login:
	@echo "Checking authentication state against $(REGISTRY)..."
	@if [ -z "$$CR_PAT" ]; then \
		echo "CR_PAT missing from environment, fetching token directly from Doppler..."; \
		doppler run -- make docker-login; \
	else \
		echo "Authenticating against $(REGISTRY) as $(GH_USERNAME)..."; \
		echo "$$CR_PAT" | docker login $(REGISTRY) \
			--username $(GH_USERNAME) \
			--password-stdin; \
		echo "Login to $(REGISTRY) succeeded."; \
	fi

# Smart wrapper for push: ensures login is valid via Doppler before pushing
docker-push:
	@if [ -z "$$CR_PAT" ]; then \
		echo "Doppler environment not active. Re-executing pipeline inside a Doppler context..."; \
		doppler run -- make docker-push; \
	else \
		make docker-build; \
		make docker-login; \
		echo "Pushing image to $(REGISTRY)..."; \
		docker push $(FULL_IMAGE); \
		echo "Image pushed successfully: $(FULL_IMAGE)"; \
	fi

clean:
	@echo "Cleaning build artefacts and local env files..."
	@cargo clean
	@rm -f $(BINARY_NAME)
	@rm -f $(ENV_FILE)
	@rm -f $(ENV_EXAMPLE)
	@echo "Clean complete."