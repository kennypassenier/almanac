# =============================================================================
# Makefile — cal-stacean (Universal Google Calendar API Gateway)
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

.PHONY: all secrets example-env build run \
	tag-major tag-minor help


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
	@echo "  secrets        Fetch secrets from Infisical and write $(ENV_FILE)"
	@echo "  example-env    Generate $(ENV_EXAMPLE) key template (values stripped)"
	@echo "  build          Compile release binary and copy to project root"
	@echo "  run            Run binary with Infisical-injected secrets"
	@echo ""
	@echo "  tag-minor      Bump minor digit and create git tag (v0.1.0 -> v0.2.0)"
	@echo "  tag-major      Bump major digit and create git tag (v1.2.3 -> v2.0.0)"
	@echo ""
	@echo "  FULL_IMAGE   = $(FULL_IMAGE)"
	@echo "  LATEST_IMAGE = $(LATEST_IMAGE)"
	@echo ""


# =============================================================================
# LOCAL MANAGEMENT TARGETS
# =============================================================================

secrets:
	@echo "Fetching secrets from Infisical..."
	@infisical export --env=dev --format=dotenv > $(ENV_FILE)
	@echo "$(ENV_FILE) successfully generated."

example-env:
	@echo "Generating $(ENV_EXAMPLE) template..."
	@infisical secrets generate-example-env > $(ENV_EXAMPLE)
	@echo "$(ENV_EXAMPLE) successfully generated."

build: secrets example-env
	@echo "Building Rust binary in release mode..."
	@cargo build --release
	@cp target/release/$(BINARY_NAME) ./$(BINARY_NAME)
	@echo "Build complete. Binary placed at: ./$(BINARY_NAME)"

# Run the binary with .env loaded (Infisical injects secrets in CI/CD; locally, use 'secrets' target)
run: build
	@echo "Starting daemon..."
	./$(BINARY_NAME)





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


# tag-major — increment the major digit and reset minor/patch (v1.2.3 -> v2.0.0).
tag-major:
	$(eval NEXT_TAG := v$(shell echo $$(($(_MAJOR) + 1))).0.0)
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."

# tag-minor — increment the minor digit and reset patch (v0.1.3 -> v0.2.0).
tag-minor:
	$(eval NEXT_TAG := v$(_MAJOR).$(shell echo $$(($(_MINOR) + 1))).0)
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."

	@echo "Cleaning build artefacts and local env files..."
	@cargo clean
	@rm -f $(BINARY_NAME)
	@rm -f $(ENV_FILE)
	@rm -f $(ENV_EXAMPLE)
	@echo "Clean complete."