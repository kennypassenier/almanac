# =============================================================================
# Makefile — almanac
# =============================================================================
#
# Walking-skeleton scope only (milestone L0): build/run/clean/tag.
# Secrets wiring via Latch (`latch run --`) lands in milestone L1 — see
# docs/REALIZATION_PLAN.md.

BINARY_NAME = almanac

.PHONY: build run clean tag-major tag-minor tag-patch help

help:
	@echo ""
	@echo "almanac — available make targets"
	@echo "---------------------------------"
	@echo "  build       Compile release binary"
	@echo "  run         Build and run the binary"
	@echo "  clean       Remove build artifacts"
	@echo ""
	@echo "  tag-patch   Bump Cargo.toml + tag (0.1.0 -> 0.1.1)"
	@echo "  tag-minor   Bump Cargo.toml + tag (0.1.0 -> 0.2.0)"
	@echo "  tag-major   Bump Cargo.toml + tag (1.2.3 -> 2.0.0)"
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

# M8: Cargo.toml is the single source of the version, and the tag
# follows it. These targets used to create a tag without touching
# Cargo.toml, which is how the binary ended up reporting 0.1.0 while
# the only tag said v0.0.1 — harmless until a self-updater has to
# compare its own version against the latest release, at which point it
# either never updates or updates on every poll. check-version.sh now
# fails the build on any disagreement, so the bump has to happen here.
CURRENT_VERSION = $(shell grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
_VER_PARTS      = $(subst ., ,$(CURRENT_VERSION))
_MAJOR          = $(word 1,$(_VER_PARTS))
_MINOR          = $(word 2,$(_VER_PARTS))

define bump
	@if ! git diff --quiet || ! git diff --cached --quiet; then \
		echo "working tree is dirty — commit or stash before tagging" >&2; exit 1; \
	fi
	sed -i '0,/^version = /s/^version = .*/version = "$(1)"/' Cargo.toml
	cargo update --workspace --quiet
	git add Cargo.toml Cargo.lock
	git commit -m "chore: release v$(1)"
	git tag v$(1)
	@echo "Cargo.toml is now $(1) and tag v$(1) exists locally."
	@echo "Run 'git push && git push --tags' to publish, then ./scripts/sign-release.sh"
endef

tag-major:
	$(call bump,$(shell echo $$(($(_MAJOR) + 1))).0.0)

tag-minor:
	$(call bump,$(_MAJOR).$(shell echo $$(($(_MINOR) + 1))).0)

tag-patch:
	$(call bump,$(_MAJOR).$(_MINOR).$(shell echo $$(($(word 3,$(_VER_PARTS)) + 1))))
