# syntax=docker/dockerfile:1.7
#
# Production image for maidan-server.
#
# Multi-stage:
#   1. `planner` extracts the dependency manifest via cargo-chef so the
#      builder stage can cache deps independently of source.
#   2. `cacher` builds all the dependencies. Cached across source edits.
#   3. `builder` compiles maidan-server in release mode.
#   4. `runtime` is a minimal glibc image (Chainguard glibc-dynamic) holding only the binary.
#
# The build names its features (`--no-default-features --features otel`), so
# the image holds exactly what a release ships whatever the crate's defaults
# become, and never the test-only `bootstrap` seed routes. `otel` is named
# because without it the image silently ignores `OTLP_ENDPOINT`.

ARG RUST_VERSION=1.99
ARG DEBIAN_CODENAME=bookworm

# The public landing is a Vite app. Build it before the Rust compile so the
# binary can embed the bundle. The Rust builder skips its own npm build.
FROM node:22-bookworm-slim AS landing
WORKDIR /web
COPY web/landing/package.json web/landing/package-lock.json ./
RUN npm ci
COPY web/landing/ ./
ENV LANDING_OUT=/landing-dist
RUN npm run build

# The console (web/console, served at /ui) is built the same way, beside the landing.
FROM node:22-bookworm-slim AS console
WORKDIR /web
COPY web/console/package.json web/console/package-lock.json ./
RUN npm ci
COPY web/console/ ./
ENV CONSOLE_OUT=/console-dist
RUN npm run build

FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS chef
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.71 --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS cacher
COPY --from=planner /app/recipe.json recipe.json
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN cargo chef cook --release --recipe-path recipe.json --bin maidan-server --no-default-features --features otel

FROM chef AS builder
# Release tag baked into the binary's reported version (see version() in
# maidan-server). Placing the ENV right before the build means a changed tag
# invalidates this layer and forces a recompile, so the image never reports a
# stale version. Empty by default, in which case version() falls back to the
# crate version.
ARG MAIDAN_VERSION=
ENV MAIDAN_VERSION=$MAIDAN_VERSION
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
COPY . .
COPY --from=landing /landing-dist crates/maidan-server/static/landing/dist
COPY --from=console /console-dist crates/maidan-server/static/console/dist
ENV MAIDAN_SKIP_LANDING_BUILD=1
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --bin maidan-server --no-default-features --features otel

# Chainguard's glibc-dynamic: glibc, libgcc, CA certificates and a nonroot
# user, no shell and no OpenSSL. Distroless's cc-debian12 carried a libssl3 the
# release scan refused (CVE-2026-84782) for weeks after Debian's fix, though no
# Maidan binary links OpenSSL. Pinned by digest; bump it to take patches.
FROM cgr.dev/chainguard/glibc-dynamic@sha256:671a621016fb33812136dea6182b1db957996d3c8c618187aaf86abd9b14af53 AS runtime
# The official MCP registry accepts an OCI package only when the image names
# the server it belongs to; it must match listings/registry/server.json.
LABEL io.modelcontextprotocol.server.name="io.github.david-engelmann/maidan" \
      org.opencontainers.image.source="https://github.com/david-engelmann/maidan" \
      org.opencontainers.image.licenses="MIT"
WORKDIR /
COPY --from=builder /app/target/release/maidan-server /usr/local/bin/maidan-server
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/maidan-server"]
