# syntax=docker/dockerfile:1.7
#
# Production image for maidan-server.
#
# Multi-stage:
#   1. `planner` extracts the dependency manifest via cargo-chef so the
#      builder stage can cache deps independently of source.
#   2. `cacher` builds all the dependencies. Cached across source edits.
#   3. `builder` compiles maidan-server in release mode.
#   4. `runtime` is a distroless image holding only the binary.
#
# Bootstrap seed routes (`POST /workspaces`) are omitted unless
# `MAIDAN_ENABLE_BOOTSTRAP=1` (CI smoke / local seed). Production deploys
# should leave the build arg at 0.

ARG RUST_VERSION=1.91
ARG DEBIAN_CODENAME=bookworm
ARG MAIDAN_ENABLE_BOOTSTRAP=0

FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS chef
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.71 --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS cacher
ARG MAIDAN_ENABLE_BOOTSTRAP=0
COPY --from=planner /app/recipe.json recipe.json
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN if [ "$MAIDAN_ENABLE_BOOTSTRAP" = "1" ]; then \
      cargo chef cook --release --recipe-path recipe.json --bin maidan-server; \
    else \
      cargo chef cook --release --recipe-path recipe.json --bin maidan-server --no-default-features; \
    fi

FROM chef AS builder
ARG MAIDAN_ENABLE_BOOTSTRAP=0
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
COPY . .
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN if [ "$MAIDAN_ENABLE_BOOTSTRAP" = "1" ]; then \
      cargo build --release --bin maidan-server; \
    else \
      cargo build --release --bin maidan-server --no-default-features; \
    fi

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /
COPY --from=builder /app/target/release/maidan-server /usr/local/bin/maidan-server
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/maidan-server"]
