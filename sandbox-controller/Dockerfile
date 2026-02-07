# syntax=docker/dockerfile:1

# --- Chef stage: install cargo-chef ---
FROM rust:1.88-slim AS chef
RUN cargo install cargo-chef
WORKDIR /app

# --- Planner stage: generate dependency recipe ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Builder stage: build dependencies then source ---
FROM chef AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Cook dependencies (cached unless Cargo.toml/lock change)
# Limit parallel jobs to reduce peak memory usage during cross-compilation
ENV CARGO_BUILD_JOBS=2
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

# Build the actual binary
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release && \
    cp target/release/sandbox-controller /app/sandbox-controller

# --- Runtime stage ---
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/sandbox-controller /
EXPOSE 8080 9090
CMD ["/sandbox-controller"]
