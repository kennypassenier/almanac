# =============================================================================
# Dockerfile — almanac
#
# For the homelab-v2 path only. The standalone LXC runs the binary
# under systemd (deploy/almanac.service), because M10's self-update
# cannot work inside a container: a replaced binary lives in the
# writable layer and is discarded on the next recreation.
#
# Build:
#   docker build -t ghcr.io/kennypassenier/almanac:v0.1.0 .
# =============================================================================

FROM rust:1.97-slim AS builder
WORKDIR /app
COPY . .
# --locked keeps the build reproducible against the committed lockfile.
RUN cargo build --release --locked


# trixie, not bookworm: the rust builder image moved to a trixie base,
# so the binary it produces links against GLIBC_2.39 and a bookworm
# runtime (2.36) cannot start it — `version 'GLIBC_2.39' not found`.
# Trixie is also what the fleet actually runs; CT 112 is on 2.41.
# Pinning the pair matters more than which of the two moves: a builder
# and a runtime from different Debian releases is a broken image that
# builds successfully.
FROM debian:trixie-slim AS runtime

# ca-certificates only: reqwest uses rustls (AR5/AR6), so no OpenSSL
# runtime library is needed. wget is for the compose healthcheck.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/almanac /usr/local/bin/almanac

# Everything mutable lives here and must be a volume in compose. The
# previous version of this file had neither a WORKDIR nor a volume, so
# the binary looked for profiles at "/profiles" and the token store
# was destroyed by every image bump — found by the pre-deployment
# critic re-run, 2026-08-28.
RUN mkdir -p /var/lib/almanac /etc/almanac/profiles
WORKDIR /var/lib/almanac

ENV ALMANAC_PROFILES_DIR=/etc/almanac/profiles \
    ALMANAC_DATA_DIR=/var/lib/almanac \
    ALMANAC_JOURNAL=/var/lib/almanac/journal.jsonl \
    ALMANAC_TOKEN_STORE=/var/lib/almanac/tokens.json

EXPOSE 8080
CMD ["/usr/local/bin/almanac"]
