# =============================================================================
# Makefile — almanac
# =============================================================================
#
# Build/run/clean plus the release entry point. Secrets reach the
# binary through Latch (`latch run --`), never through this file.

BINARY_NAME = almanac

.PHONY: build run clean release release-dry help

help:
	@echo ""
	@echo "almanac — available make targets"
	@echo "---------------------------------"
	@echo "  build       Compile release binary"
	@echo "  run         Build and run the binary"
	@echo "  clean       Remove build artifacts"
	@echo ""
	@echo "  release     make release VERSION=x.y.z — gate, bump, build and"
	@echo "              publish locally (chassis release; needs chassis >= 3.0.0)"
	@echo "  release-dry make release-dry VERSION=x.y.z — the same gate and builds,"
	@echo "              stops before any commit, tag, push or upload"
	@echo ""

build:
	cargo build --release

run: build
	./target/release/$(BINARY_NAME)

clean:
	cargo clean

# -----------------------------------------------------------------------------
# Releasing — `chassis release <version>` (chassis-rs >= 3.0.0) does the
# whole chain on this machine: the gate, the Cargo.toml + CHANGELOG bump
# and commit, the local tag, the static musl binary + SHA256SUMS, the
# image, and only then the push, `docker push`, `gh release create` and
# scripts/sign-release.sh. Nothing is built on GitHub Actions.
#
# M8: Cargo.toml is the single source of the version, and the tag
# follows it. The old tag-* targets once created a tag without touching
# Cargo.toml, which is how the binary ended up reporting 0.1.0 while the
# only tag said v0.0.1 — harmless until a self-updater has to compare
# its own version against the latest release. `chassis release` bumps
# both in one step, and scripts/check-version.sh still fails the build
# on any disagreement.
# -----------------------------------------------------------------------------

define require_version
	@test -n "$(VERSION)" || { echo "usage: make $@ VERSION=x.y.z" >&2; exit 1; }
	@case "$(VERSION)" in \
		[0-9]*.[0-9]*.[0-9]*) ;; \
		*) echo "VERSION must be plain x.y.z (no leading v), got '$(VERSION)'" >&2; exit 1 ;; \
	esac
endef

release:
	$(require_version)
	./scripts/check-ci.sh
	chassis release $(VERSION)

release-dry:
	$(require_version)
	chassis release $(VERSION) --dry-run
