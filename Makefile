# =============================================================================
# Makefile — almanac
# =============================================================================
#
# Walking-skeleton scope only (milestone L0): build/run/clean/tag.
# Secrets wiring via Latch (`latch run --`) lands in milestone L1 — see
# docs/REALIZATION_PLAN.md.

BINARY_NAME = almanac

.PHONY: build run clean tag-major tag-minor help

help:
	@echo ""
	@echo "almanac — available make targets"
	@echo "---------------------------------"
	@echo "  build       Compile release binary"
	@echo "  run         Build and run the binary"
	@echo "  clean       Remove build artifacts"
	@echo ""
	@echo "  tag-minor   Bump minor digit and create a git tag (v0.1.0 -> v0.2.0)"
	@echo "  tag-major   Bump major digit and create a git tag (v1.2.3 -> v2.0.0)"
	@echo ""

build:
	cargo build --release

run: build
	./target/release/$(BINARY_NAME)

clean:
	cargo clean

# -----------------------------------------------------------------------------
# Versioning helpers — create the next git tag locally without pushing.
# Run 'git push --tags' to publish it.
# -----------------------------------------------------------------------------

CURRENT_TAG = $(shell git describe --tags --abbrev=0 2>/dev/null || echo "v0.0.0")
_VER_PARTS  = $(subst ., ,$(patsubst v%,%,$(CURRENT_TAG)))
_MAJOR      = $(word 1,$(_VER_PARTS))
_MINOR      = $(word 2,$(_VER_PARTS))

tag-major:
	$(eval NEXT_TAG := v$(shell echo $$(($(_MAJOR) + 1))).0.0)
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."

tag-minor:
	$(eval NEXT_TAG := v$(_MAJOR).$(shell echo $$(($(_MINOR) + 1))).0)
	@echo "Current tag: $(CURRENT_TAG) -> New tag: $(NEXT_TAG)"
	git tag $(NEXT_TAG)
	@echo "Tag $(NEXT_TAG) created locally. Run 'git push --tags' to publish it."
