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

# IMAGE_TAG is derived automatically from the most recent git tag.
# Examples:
#   v0.2.0              — HEAD is exactly at tag v0.2.0
#   v0.2.0-4-gabcdef    — 4 commits past v0.2.0 (pre-release build)
#   abcdef              — no tags exist yet; falls back to short SHA
# To cut a new release, run one of:
#   make tag-patch      — bumps the patch digit  (v0.1.0 -> v0.1.1)
#   make tag-minor      — bumps the minor digit  (v0.1.0 -> v0.2.0)
# then run 'make docker-push' as normal.
IMAGE_TAG     = $(shell git describe --tags --always 2>/dev/null || echo "dev")

# Versioned reference used for this specific build.
FULL_IMAGE    = $(REGISTRY)/$(GH_USERNAME)/$(IMAGE_NAME):$(IMAGE_TAG)
# Floating 'latest' reference updated on every push.
LATEST_IMAGE  = $(REGISTRY)/$(GH_USERNAME)/$(IMAGE_NAME):latest

.PHONY: all secrets example-env build run clean \
        docker-build docker-login docker-push \
        tag-patch tag-minor help


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
	@echo "  docker-push    Build and push to GHCR (add AUTO_TAG=1 to bump patch first)"
	@echo "  tag-patch      Bump patch digit and create git tag (v0.1.0 -> v0.1.1)"
	@echo "  tag-minor      Bump minor digit and create git tag (v0.1.0 -> v0.2.0)"
	@echo ""
	@echo "  FULL_IMAGE   = $(FULL_IMAGE)"
	@echo "  LATEST_IMAGE = $(LATEST_IMAGE)"
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
		--tag $(LATEST_IMAGE) \
		.
	@echo "Docker image built and tagged: $(FULL_IMAGE) and $(LATEST_IMAGE)"

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

# Set AUTO_TAG=1 on the command line to automatically bump the patch digit
# and create a new git tag before the image is built and pushed.
# Default is 0 (off) so a plain 'make docker-push' never mutates tags.
#
# Usage:
#   make docker-push             # push current version, no tag change
#   make docker-push AUTO_TAG=1  # bump patch (v0.1.0 -> v0.1.1) then push
AUTO_TAG = 1

# Smart wrapper for push: ensures login is valid via Doppler before pushing
docker-push:
	@if [ -z "$$CR_PAT" ]; then \
		echo "Doppler environment not active. Re-executing pipeline inside a Doppler context..."; \
		doppler run -- make docker-push AUTO_TAG=$(AUTO_TAG); \
	else \
		if [ "$(AUTO_TAG)" = "1" ]; then \
			echo "AUTO_TAG=1 — bumping patch version before push..."; \
			make tag-patch; \
		fi; \
		make docker-build; \
		make docker-login; \
		echo "Pushing image to $(REGISTRY)..."; \
		docker push $(FULL_IMAGE); \
		docker push $(LATEST_IMAGE); \
		echo "Image pushed successfully: $(FULL_IMAGE) and $(LATEST_IMAGE)"; \
	fi

# ---------------------------------------------------------------------------
# Versioning helpers — create the next git tag without pushing it to remote.
# Run 'git push --tags' or let docker-push carry the image after tagging.
# ---------------------------------------------------------------------------

# Determine the highest existing semver tag (defaults to v0.0.0 if none exist).
CURRENT_TAG   = $(shell git describe --tags --abbrev=0 2>/dev/null || echo "v0.0.0")
# Strip the leading 'v' and split into major, minor, patch components.
_VER_PARTS    = $(subst ., ,$(patsubst v%,%,$(CURRENT_TAG)))
_MAJOR        = $(word 1,$(_VER_PARTS))
_MINOR        = $(word 2,$(_VER_PARTS))
_PATCH        = $(word 3,$(_VER_PARTS))

# tag-patch — increment the patch digit (v0.1.0 -> v0.1.1).
tag-patch:
	$(eval NEXT_TAG := v$(_MAJOR).$(_MINOR).$(shell echo $$(($(_PATCH) + 1))))
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."

# tag-minor — increment the minor digit and reset patch (v0.1.3 -> v0.2.0).
tag-minor:
	$(eval NEXT_TAG := v$(_MAJOR).$(shell echo $$(($(_MINOR) + 1))).0)
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."

clean:
	@echo "Cleaning build artefacts and local env files..."
	@cargo clean
	@rm -f $(BINARY_NAME)
	@rm -f $(ENV_FILE)
	@rm -f $(ENV_EXAMPLE)
	@echo "Clean complete."