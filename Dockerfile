# =============================================================================
# Dockerfile — cal-stacean
# Multi-stage build: compile in a full Rust toolchain image, then copy only
# the statically-linked binary and the required runtime libraries into a
# minimal Debian bookworm-slim image for production deployment.
#
# Build command (run from the project root):
#   docker build -t ghcr.io/<gh-username>/cal-stacean:latest .
#
# Stage layout:
#   builder  — rust:slim    — compiles the release binary
#   runtime  — debian:slim  — runs the binary; no compiler, no source code
# =============================================================================


# -----------------------------------------------------------------------------
# Stage 1 — Builder
#
# Uses the official Rust slim image which includes cargo, rustc, and the
# standard library.  pkg-config and libssl-dev are required at compile time
# because reqwest links against OpenSSL (or its dev headers) during the build.
# The compiled binary is placed at /app/target/release/cal-stacean.
# -----------------------------------------------------------------------------
FROM rust:1.87-slim AS builder

# Install compile-time dependencies.
# - pkg-config   : allows the build system to locate system libraries
# - libssl-dev   : OpenSSL development headers required by reqwest
# The apt cache is removed in the same layer to keep the builder stage lean.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set the working directory for all subsequent COPY and RUN instructions.
WORKDIR /app

# Copy the full workspace into the builder stage.
# .dockerignore should exclude target/, .env, *.env.example, and other
# non-source artefacts to keep the build context small and reproducible.
COPY . .

# Compile the application in release mode.
# --locked ensures that Cargo.lock is respected exactly, producing a
# reproducible build identical to what was tested locally.
RUN cargo build --release --locked


# -----------------------------------------------------------------------------
# Stage 2 — Runtime
#
# Starts from a minimal Debian bookworm-slim base.  Only the compiled binary,
# the application configuration, and the runtime TLS libraries are present.
# No compiler, no source code, no development headers.
#
# Runtime dependency rationale:
# - ca-certificates  : provides the system CA bundle so that outbound HTTPS
#                      requests to Google APIs (OAuth2 token endpoint, Calendar
#                      API) can be verified against a trusted root CA.
# - libssl3          : the OpenSSL shared library required at runtime by the
#                      reqwest TLS backend (dynamically linked in the binary).
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies and clean up the apt cache in a single layer.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create a dedicated directory for application configuration.
# Keeping configuration separate from the binary makes it straightforward to
# override config.toml at runtime via a bind-mount or Kubernetes ConfigMap
# without rebuilding the image.
RUN mkdir -p /etc/cal-stacean

# Copy the compiled release binary from the builder stage.
# The binary is placed on the standard system PATH so it can be invoked
# without an explicit path prefix.
COPY --from=builder /app/target/release/cal-stacean /usr/local/bin/cal-stacean

# Copy the application configuration into the well-known config directory.
# This bakes a default configuration into the image.  Operators can override
# it at runtime by mounting an alternative config.toml at the same path.
COPY config.toml /etc/cal-stacean/config.toml

# Tell the application where to find its configuration file at runtime.
# The binary reads config.toml from the current working directory by default;
# this variable can be used to point it at the /etc path instead if the
# application is updated to honour CONFIG_PATH.
ENV CONFIG_PATH=/etc/cal-stacean/config.toml

# Expose the port the Axum server listens on.
# This is documentation only; actual port binding is done at container run
# time with -p 8080:8080 or via Kubernetes Service/Ingress configuration.
EXPOSE 8080

# Set the default command to run the daemon.
# Using CMD (rather than ENTRYPOINT) allows the command to be overridden
# easily when running the container for debugging or one-off tasks.
CMD ["/usr/local/bin/cal-stacean"]
