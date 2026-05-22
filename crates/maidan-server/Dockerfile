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

ARG RUST_VERSION=1.88
ARG DEBIAN_CODENAME=bookworm

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
RUN cargo chef cook --release --recipe-path recipe.json --bin maidan-server

FROM chef AS builder
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
COPY . .
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --bin maidan-server

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /
COPY --from=builder /app/target/release/maidan-server /usr/local/bin/maidan-server
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/maidan-server"]
