# =============================================================================
# Makefile — almanac
# =============================================================================
#
# Build/run/clean plus the release entry point. Secrets reach the
# binary through Latch (`latch run --`), never through this file.

BINARY_NAME = almanac

.PHONY: build run clean live-test release release-dry help

help:
	@echo ""
	@echo "almanac — available make targets"
	@echo "---------------------------------"
	@echo "  build       Compile release binary"
	@echo "  run         Build and run the binary"
	@echo "  clean       Remove build artifacts"
	@echo "  live-test   Run the live suites (real calendar) under latch run --"
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
# Live tests (T19) — the two #[ignore]d suites are the only proof that
# the Google Calendar round-trip (K1), the upsert that stops a
# redelivery duplicating (K2) and both power-loss drills (AR16) work.
# They write to the throwaway calendar ALMANAC_TEST_CALENDAR_ID (never
# the household one) with the service account's real credentials, all
# of which come from Latch's default environment, like every other
# local run of them. Run from a checkout linked to Latch (`latch init`).
#
# Nothing runs these unattended any more: the nightly GitHub Actions
# schedule is gone with the rest of the workflows. Run this by hand,
# before a release and after anything touching the Calendar client,
# the upsert key or the journal.
#
# Refuses rather than passing vacuously when the calendar id is
# missing: a green run that tested nothing is worse than a red one.
# -----------------------------------------------------------------------------

live-test:
	@latch run -- sh -c 'test -n "$$ALMANAC_TEST_CALENDAR_ID"' || { \
		echo "ALMANAC_TEST_CALENDAR_ID is not set in Latch (or this checkout is not linked to it)." >&2; \
		echo "What now: 'latch edit .env' and set it to the throwaway test calendar" >&2; \
		echo "(cargo run --example create_test_calendar makes one), then" >&2; \
		echo "'latch commit .env && latch push'. Never the household calendar." >&2; \
		exit 1; }
	latch run -- cargo test --test calendar_e2e -- --ignored --nocapture
	latch run -- cargo test --test power_loss_drill -- --ignored --nocapture

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

# Kenny, 2026-09-29 ("Bij elke release"): the live suites no longer run
# nightly on GitHub, so every release runs them first, against the real
# calendar, before anything is tagged or built.
release:
	$(require_version)
	$(MAKE) live-test
	chassis release $(VERSION)

release-dry:
	$(require_version)
	chassis release $(VERSION) --dry-run
