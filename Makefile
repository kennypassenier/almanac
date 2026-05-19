# =============================================================================
# Makefile — cal-stacean
#
# Targets are divided into two logical groups:
#
#   Local management  — build, run, and clean the binary on the dev machine
#   Container pipeline — build, authenticate, and push the Docker image to GHCR
#
# Usage:
#   make secrets        Fetch secrets from Doppler into a local .env file
#   make example-env    Generate a .env.example key template (values stripped)
#   make build          Compile a release binary and copy it to the project root
#   make run            Run the binary with secrets injected via Doppler
#   make clean          Remove all build artefacts and local env files
#
#   make docker-build   Build the multi-stage Docker image and tag it for GHCR
#   make docker-login   Authenticate the current shell against ghcr.io
#   make docker-push    Build and push the image to the remote registry
# =============================================================================


# -----------------------------------------------------------------------------
# Structural variables
# -----------------------------------------------------------------------------

# Local binary and environment file names.
BINARY_NAME   = cal-stacean
ENV_FILE      = .env
ENV_EXAMPLE   = .env.example

# Container registry coordinates.
REGISTRY      = ghcr.io
GH_USERNAME   = kennypassenier
IMAGE_NAME    = cal-stacean
IMAGE_TAG     = v0.1.0

# Fully-qualified image reference used in all Docker commands.
FULL_IMAGE    = $(REGISTRY)/$(GH_USERNAME)/$(IMAGE_NAME):$(IMAGE_TAG)


# -----------------------------------------------------------------------------
# Declared phony targets
# -----------------------------------------------------------------------------
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

# secrets — fetch live secrets from Doppler and write them to .env.
secrets:
	@echo "Fetching secrets from Doppler..."
	@doppler secrets download --format=env --no-file > $(ENV_FILE)
	@echo "$(ENV_FILE) successfully generated."

# example-env — generate an .env.example file containing only key names.
example-env:
	@echo "Generating $(ENV_EXAMPLE) template..."
	@doppler secrets download --format=env --no-file \
		| sed 's/=.*$$/=your_value_here/' > $(ENV_EXAMPLE)
	@echo "$(ENV_EXAMPLE) successfully generated."

# build — compile the release binary and copy it to the project root.
build: secrets example-env
	@echo "Building Rust binary in release mode..."
	@cargo build --release
	@cp target/release/$(BINARY_NAME) ./$(BINARY_NAME)
	@echo "Build complete. Binary placed at: ./$(BINARY_NAME)"

# run — launch the daemon from the project-root binary.
run: build
	@echo "Starting daemon with Doppler-injected environment..."
	@doppler run -- ./$(BINARY_NAME)

# clean — remove all build artefacts and local sensitive files.
clean:
	@echo "Cleaning build artefacts and local env files..."
	@cargo clean
	@rm -f $(BINARY_NAME)
	@rm -f $(ENV_FILE)
	@rm -f $(ENV_EXAMPLE)
	@echo "Clean complete."


# =============================================================================
# CONTAINER PIPELINE TARGETS
# =============================================================================

# docker-build — build the multi-stage Docker image and tag it for GHCR.
docker-build:
	@echo "Building Docker image: $(FULL_IMAGE)"
	docker build \
		--tag $(FULL_IMAGE) \
		.
	@echo "Docker image built and tagged: $(FULL_IMAGE)"

# docker-login — authenticate the current shell session against ghcr.io.
docker-login:
	@echo "Authenticating against $(REGISTRY) as $(GH_USERNAME)..."
	@echo "$$CR_PAT" | docker login $(REGISTRY) \
		--username $(GH_USERNAME) \
		--password-stdin
	@echo "Login to $(REGISTRY) succeeded."

# docker-push — build the image locally and push it to GHCR.
docker-push: docker-build
	@echo "Pushing image to $(REGISTRY)..."
	docker push $(FULL_IMAGE)
	@echo "Image pushed successfully: $(FULL_IMAGE)"