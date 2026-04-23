# syntax=docker/dockerfile:1.6
ARG RUST_VERSION=1

FROM rust:${RUST_VERSION}-slim AS builder
WORKDIR /src
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config cmake \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/npm-docker-sync /tmp/npm-docker-sync

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/npm-docker-sync /usr/local/bin/npm-docker-sync
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/npm-docker-sync"]
CMD ["--config", "/etc/npm-docker-sync/config.toml"]
